import 'package:flutter_test/flutter_test.dart';
import 'package:android_verifier/verifier.dart';

void main() {
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
