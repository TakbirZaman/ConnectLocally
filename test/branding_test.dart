import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:localconnect/brand.dart';
import 'package:localconnect/phone_control_disclosure.dart';

void main() {
  testWidgets('Creator credit opens only on tap and fits narrow enlarged text', (tester) async {
    var calls = 0;
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: MediaQuery(
      data: const MediaQueryData(textScaler: TextScaler.linear(2)),
      child: SizedBox(width: 280, child: CreatorCredit(openProfile: () async { calls++; })),
    ))));
    expect(calls, 0);
    expect(tester.takeException(), isNull);
    await tester.tap(find.byType(TextButton));
    await tester.pump();
    expect(calls, 1);
  });

  testWidgets('Missing browser gives a copy-link fallback', (tester) async {
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: CreatorCredit(
      openProfile: () async => throw StateError('No browser'),
    ))));
    await tester.tap(find.byType(TextButton));
    await tester.pumpAndSettle();
    expect(find.text('Copy link'), findsOneWidget);
    expect(find.textContaining('Could not open LinkedIn'), findsOneWidget);
  });

  testWidgets('Accessibility disclosure requires affirmative consent', (tester) async {
    bool? accepted;
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => Scaffold(
      body: TextButton(onPressed: () async {
        accepted = await showPhoneControlDisclosure(context);
      }, child: const Text('Configure')),
    ))));
    await tester.tap(find.text('Configure'));
    await tester.pumpAndSettle();
    expect(accepted, isNull);
    expect(find.textContaining('focused editable field'), findsOneWidget);
    await tester.tap(find.text('Not now'));
    await tester.pumpAndSettle();
    expect(accepted, false);
    await tester.tap(find.text('Configure'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Agree and open settings'));
    await tester.pumpAndSettle();
    expect(accepted, true);
  });
}
