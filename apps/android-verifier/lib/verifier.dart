import 'dart:convert';
import 'dart:collection';

import 'package:flutter/services.dart';

import 'src/rust/api/verifier.dart' as api;
import 'src/rust/frb_generated.dart';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

class VerifierFailure implements Exception {
  final String code;
  const VerifierFailure(this.code);
}

Map<String, dynamic> decodeReply(String raw) {
  final envelope = jsonDecode(raw) as Map<String, dynamic>;
  if (envelope['ok'] != true) {
    throw VerifierFailure((envelope['error'] as Map)['code'] as String);
  }
  return envelope['value'] as Map<String, dynamic>;
}

class TextWindow {
  static const limit = 64 * 1024;
  final Queue<String> _chunks = Queue();
  int bytes = 0;
  bool truncated = false;
  int lastSequence = 0;
  void append(int sequence, String text) {
    if (sequence <= lastSequence) return;
    lastSequence = sequence;
    final length = utf8.encode(text).length;
    if (length > 4096) throw const VerifierFailure('native_protocol');
    _chunks.add(text);
    bytes += length;
    while (bytes > limit) {
      bytes -= utf8.encode(_chunks.removeFirst()).length;
      truncated = true;
    }
  }

  String get text => _chunks.join();
  void clear() {
    _chunks.clear();
    bytes = 0;
    truncated = false;
  }
}

class Verifier {
  static const platform = MethodChannel(
    'io.github.naza3.nexa.verifier/platform_v1',
  );
  String? epoch;
  static Future<void>? _rustInitialization;
  Future<Map<String, dynamic>> initialize() async {
    await (_rustInitialization ??= RustLib.init(
      externalLibrary: ExternalLibrary.open('libnexa_device_verifier.so'),
    ));
    for (var i = 0; i < 120; i++) {
      try {
        final state = decodeReply(await api.verifierOpen());
        epoch = state['host_epoch'] as String;
        return state;
      } on VerifierFailure catch (e) {
        if (e.code != 'unavailable') rethrow;
        await Future<void>.delayed(const Duration(milliseconds: 250));
      }
    }
    throw const VerifierFailure('native_bootstrap_failed');
  }

  Future<Map<String, dynamic>> snapshot() async =>
      decodeReply(await api.verifierSnapshot(epoch: epoch!));
  Future<Map<String, dynamic>?> importCandidate() async {
    final token = await platform.invokeMethod<String>('pick_candidate', {
      'epoch': epoch,
    });
    if (token == null) return null;
    try {
      return decodeReply(
        await api.candidateImport(epoch: epoch!, selectionToken: token),
      );
    } finally {
      await platform.invokeMethod<void>('cancel_selection', {
        'epoch': epoch,
        'selection_token': token,
      });
    }
  }

  Future<Map<String, dynamic>> start(String suite) async =>
      decodeReply(await api.suiteStart(epoch: epoch!, suiteId: suite));
  Future<Map<String, dynamic>> remove() async =>
      decodeReply(await api.candidateRemove(epoch: epoch!));
  Future<Map<String, dynamic>> next(String operation, int? ack) async =>
      decodeReply(
        await api.operationNext(
          epoch: epoch!,
          operationId: operation,
          ackSequence: ack == null ? null : BigInt.from(ack),
        ),
      );
  Future<void> cancel(String operation) async {
    decodeReply(
      await api.operationCancel(epoch: epoch!, operationId: operation),
    );
  }

  Future<String?> export(String operation) async {
    final descriptor = decodeReply(
      await api.reportPrepare(epoch: epoch!, operationId: operation),
    );
    return platform.invokeMethod<String>('export_report', {
      'epoch': epoch,
      'report_token': descriptor['report_token'],
    });
  }

  Future<void> licenses() => platform.invokeMethod<void>('show_licenses');
}
