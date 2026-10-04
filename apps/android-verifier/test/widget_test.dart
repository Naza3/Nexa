import 'package:flutter_test/flutter_test.dart';
import 'package:flutter/widgets.dart';
import 'package:android_verifier/main.dart';
import 'package:android_verifier/src/rust/frb_generated.dart';
import 'package:android_verifier/verifier.dart';

class _CancelApi extends RustLibApi {
  final List<String> cancelled = [];
  @override
  Future<String> crateApiVerifierOperationCancel({
    required String epoch,
    required String operationId,
  }) async {
    cancelled.add(operationId);
    return '{"ok":true,"value":{"state":"stopping"}}';
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  testWidgets(
    'actual observer ignores focus loss and cancels hidden without replay',
    (tester) async {
      // A control-only FRB fixture; no Linux native library or Android kernel runs.
      final api = _CancelApi();
      RustLib.initMock(api: api);
      await tester.pumpWidget(const VerifierApp());
      await tester.pump();
      final dynamic state = tester.state(find.byType(VerifierPage));
      state.verifier.epoch = 'test-epoch';
      state.operation = 'test-operation';
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      await tester.pump();
      expect(api.cancelled, isEmpty);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pump();
      expect(api.cancelled, isEmpty);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
      await tester.pump();
      expect(api.cancelled, ['test-operation']);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
      await tester.pump();
      expect(api.cancelled, ['test-operation', 'test-operation']);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pump();
      expect(api.cancelled.length, 3);
      expect(state.operation, 'test-operation');
      state.operation = null;
      await tester.pumpWidget(const SizedBox.shrink());
      RustLib.dispose();
    },
  );
  test('inactive and resumed do not request lifecycle cancellation', () {
    for (final state in [
      AppLifecycleState.resumed,
      AppLifecycleState.inactive,
      AppLifecycleState.resumed,
    ]) {
      expect(lifecycleRequiresCancellation(state), isFalse);
    }
  });
  test('hidden paused and detached retain lifecycle cancellation', () {
    for (final state in [
      AppLifecycleState.hidden,
      AppLifecycleState.paused,
      AppLifecycleState.detached,
    ]) {
      expect(lifecycleRequiresCancellation(state), isTrue);
    }
  });
  test('UTF-8 window is bounded and sequence retries are idempotent', () {
    final w = TextWindow();
    for (var i = 1; i < 100; i++) {
      w.append(i, '中' * 1000);
    }
    expect(w.bytes, lessThanOrEqualTo(65536));
    expect(w.truncated, isTrue);
    final before = w.text;
    w.append(99, 'duplicate');
    expect(w.text, before);
    w.clear();
    expect(w.bytes, 0);
    expect(w.text, isEmpty);
  });
  test('oversized native fragment is rejected', () {
    expect(
      () => TextWindow().append(1, 'a' * 4097),
      throwsA(isA<VerifierFailure>()),
    );
  });
  test('error envelope exposes only reviewed code', () {
    expect(
      () => decodeReply(
        '{"ok":false,"error":{"code":"stale_handle","retry":"reopen_required"}}',
      ),
      throwsA(isA<VerifierFailure>()),
    );
  });
  test('valid JSON envelope returns the structured payload', () {
    expect(
      decodeReply(
        '{"ok":true,"value":{"production_admitted":false}}',
      )['production_admitted'],
      false,
    );
  });
}
