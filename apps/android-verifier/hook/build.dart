import 'dart:convert';
import 'dart:io';

import 'package:hooks/hooks.dart';
import 'package:code_assets/code_assets.dart';
import 'package:crypto/crypto.dart';

// Explicit locked/offline Cargo prebuild; no toolchain or engine download hook.
Future<void> main(List<String> args) async {
  await build(args, (input, output) async {
    if (!input.config.buildCodeAssets) return;
    if (input.config.code.targetOS != OS.android) {
      return; // Host-only Dart tests.
    }
    if (input.config.code.targetArchitecture != Architecture.arm64) {
      throw StateError('Only audited Android arm64 is supported');
    }
    final manifest = File.fromUri(
      input.packageRoot.resolve('build-input/native.json'),
    );
    final data =
        jsonDecode(await manifest.readAsString()) as Map<String, dynamic>;
    if (data['target'] != 'aarch64-linux-android' ||
        data['mode'] != 'release' ||
        data['library_name'] != 'libnexa_device_verifier.so') {
      throw StateError('Invalid prebuilt identity');
    }
    final library = File(data['path'] as String);
    final hash = await sha256.bind(library.openRead()).first;
    if (hash.toString() != data['sha256']) {
      throw StateError('Prebuilt hash mismatch');
    }
    final sources = data['inputs'] as Map<String, dynamic>;
    for (final entry in sources.entries) {
      final file = File.fromUri(input.packageRoot.resolve(entry.key));
      if ((await sha256.bind(file.openRead()).first).toString() !=
          entry.value) {
        throw StateError('Stale prebuilt source: ${entry.key}');
      }
      output.dependencies.add(file.uri);
    }
    output.dependencies.addAll([manifest.uri, library.uri]);
    output.assets.code.add(
      CodeAsset(
        package: input.packageName,
        name: 'nexa_device_verifier',
        linkMode: DynamicLoadingBundled(),
        file: library.uri,
      ),
    );
  });
}
