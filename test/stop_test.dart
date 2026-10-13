import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:localconnect/main.dart';
import 'package:localconnect/providers.dart';
import 'package:localconnect/src/rust/engine.dart';
import 'package:localconnect/src/rust/frb_generated.dart';

class PendingProbeApi extends Fake implements RustLibApi {
  final probe = Completer<Peer>();
  int shutdownCalls = 0;
  @override
  Future<Peer> crateApiProbe({required String address}) => probe.future;
  @override
  Future<void> crateApiShutdown() async {
    shutdownCalls++;
  }
}

void main() {
  testWidgets(
    'Stop all interrupts a busy operation and ignores its late result',
    (tester) async {
      final api = PendingProbeApi();
      RustLib.initMock(api: api);
      final controller = LocalController();
      controller.started('{"fingerprint":"test-fingerprint","port":53317}');
      await tester.pumpWidget(
        ProviderScope(
          overrides: [localProvider.overrideWith((ref) => controller)],
          child: const MaterialApp(home: Home()),
        ),
      );
      await tester.tap(find.text('Connect by IP address'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.byType(TextField));
      await tester.enterText(find.byType(TextField), '192.168.1.2:53317');
      await tester.ensureVisible(find.text('Connect'));
      await tester.tap(find.text('Connect'));
      await tester.pump();
      expect(find.byType(LinearProgressIndicator), findsOneWidget);
      await tester.tap(find.text('Stop all'));
      await tester.pump();
      expect(api.shutdownCalls, 1);
      expect(find.text('Start LocalConnect'), findsOneWidget);
      api.probe.complete(
        const Peer(
          fingerprint: 'late',
          name: 'Late peer',
          address: '192.168.1.2:53317',
        ),
      );
      await tester.pump();
      expect(find.textContaining('Pair with'), findsNothing);
      expect(find.text('Late peer'), findsNothing);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      RustLib.dispose();
    },
  );
}
