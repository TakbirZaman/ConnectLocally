import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:localconnect/main.dart';
import 'package:localconnect/providers.dart';
import 'package:localconnect/src/rust/engine.dart';
import 'package:localconnect/src/rust/frb_generated.dart';

class CodeApi extends Fake implements RustLibApi {
  bool? hostRemember;
  bool? clientRemember;
  int cancelled = 0;
  int attempts = 0;
  String? entered;
  @override
  Future<String> crateApiBeginPairingCode({required bool remember}) async {
    hostRemember = remember;
    return '{"id":"window","code":"012345","expires_in_seconds":120,"attempts":3}';
  }

  @override
  Future<void> crateApiCancelPairingCode() async {
    cancelled++;
  }

  @override
  Future<Peer> crateApiProbe({required String address}) async =>
      Peer(fingerprint: 'peer', name: 'Laptop', address: address);
  @override
  Future<String> crateApiTrustedPeers() async => '[]';
  @override
  Future<Peer> crateApiPairWithCode({
    required String address,
    required String code,
    required bool remember,
  }) async {
    attempts++;
    entered = code;
    clientRemember = remember;
    return Peer(fingerprint: 'peer', name: 'Laptop', address: address);
  }
}

void main() {
  final api = CodeApi();
  setUpAll(() => RustLib.initMock(api: api));
  tearDownAll(RustLib.dispose);
  setUp(() {
    api.hostRemember = null;
    api.clientRemember = null;
    api.cancelled = 0;
    api.attempts = 0;
    api.entered = null;
  });
  Future<CodeApi> home(WidgetTester tester) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final controller = LocalController();
    controller.started('{"fingerprint":"local","port":53317}');
    await tester.pumpWidget(
      ProviderScope(
        overrides: [localProvider.overrideWith((ref) => controller)],
        child: const MaterialApp(home: Home()),
      ),
    );
    await tester.pumpAndSettle();
    return api;
  }

  testWidgets(
    'display code defaults to temporary access and Close cancels it',
    (tester) async {
      final api = await home(tester);
      await tester.tap(find.text('Show pairing code'));
      await tester.pump(const Duration(milliseconds: 300));
      await tester.tap(find.text('Generate code'));
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text('012345'), findsOneWidget);
      expect(api.hostRemember, false);
      await tester.tap(find.text('Close code'));
      await tester.pumpAndSettle();
      expect(api.cancelled, 1);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets(
    'three digits cannot pair; six digits use temporary code authentication',
    (tester) async {
      final api = await home(tester);
      await tester.scrollUntilVisible(
        find.text('Connect by IP address'),
        200,
        scrollable: find.byType(Scrollable).first,
      );
      await tester.tap(find.text('Connect by IP address'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), '192.168.1.2:53317');
      await tester.ensureVisible(find.text('Connect'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Connect'));
      await tester.pump(const Duration(milliseconds: 300));
      final field = find.descendant(
        of: find.byType(AlertDialog),
        matching: find.byType(TextField),
      );
      await tester.enterText(field, '234');
      await tester.tap(
        find.descendant(
          of: find.byType(AlertDialog),
          matching: find.widgetWithText(FilledButton, 'Connect'),
        ),
      );
      await tester.pump(const Duration(milliseconds: 100));
      expect(api.attempts, 0);
      expect(find.text('Enter all six digits.'), findsOneWidget);
      await tester.enterText(field, '012345');
      await tester.tap(
        find.descendant(
          of: find.byType(AlertDialog),
          matching: find.widgetWithText(FilledButton, 'Connect'),
        ),
      );
      await tester.pumpAndSettle();
      expect(api.attempts, 1);
      expect(api.entered, '012345');
      expect(api.clientRemember, false);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );
}
