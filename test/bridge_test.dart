import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:localconnect/src/rust/frb_generated.dart';
import 'package:localconnect/src/rust/api.dart' as core;

void main() {
  const libraryPath = String.fromEnvironment('LOCALCONNECT_NATIVE_LIB');
  test(
    'real native bridge initializes, returns typed peer, rejects unpaired send, and stops',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'localconnect-bridge-',
      );
      await RustLib.init(externalLibrary: ExternalLibrary.open(libraryPath));
      try {
        final info =
            jsonDecode(
                  await core.initialize(
                    dataDir: '${directory.path}/state',
                    receiveDir: '${directory.path}/received',
                    name: 'Bridge test',
                    port: 0,
                  ),
                )
                as Map<String, dynamic>;
        expect(info['fingerprint'], matches(RegExp(r'^[a-f0-9]{64}$')));
        final peer = await core.probe(address: '127.0.0.1:${info['port']}');
        expect(peer.fingerprint, info['fingerprint']);
        expect(peer.name, 'Bridge test');
        final capabilities =
            jsonDecode(await core.capabilities()) as Map<String, dynamic>;
        expect(capabilities['remoteInput'], false);
        expect(capabilities['screenStreaming'], false);
        expect(jsonDecode(await core.trustedPeers()), isEmpty);
        final source = File('${directory.path}/source.bin');
        await source.writeAsBytes([1, 2, 3]);
        await expectLater(
          core.sendFile(path: source.path, peer: peer),
          throwsA(
            predicate<Object>(
              (error) => error.toString().contains('pair before sending'),
            ),
          ),
        );
      } finally {
        await core.shutdown();
        RustLib.dispose();
        await directory.delete(recursive: true);
      }
    },
    skip:
        libraryPath.isEmpty
            ? 'Build native library and pass --dart-define=LOCALCONNECT_NATIVE_LIB=ABSOLUTE_PATH'
            : false,
  );
}
