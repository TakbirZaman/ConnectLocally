import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:localconnect/main.dart';
import 'package:localconnect/providers.dart';

class FixtureController extends LocalController {
  FixtureController(LocalState fixture) {
    state = fixture;
  }
}

void main() {
  Future<void> showHome(WidgetTester tester, LocalState state) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          localProvider.overrideWith((ref) => FixtureController(state)),
        ],
        child: MaterialApp(
          theme: ThemeData(useMaterial3: true),
          builder:
              (context, child) => MediaQuery(
                data: MediaQuery.of(
                  context,
                ).copyWith(textScaler: const TextScaler.linear(1.4)),
                child: child!,
              ),
          home: const Home(),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets('onboarding has one clear start action on a narrow phone', (
    tester,
  ) async {
    await showHome(tester, const LocalState());
    expect(find.text('Start LocalConnect'), findsOneWidget);
    expect(find.byType(NavigationBar), findsNothing);
    expect(tester.takeException(), isNull);
  });

  testWidgets('tabs separate tasks and retain immediate session Stop', (
    tester,
  ) async {
    await showHome(
      tester,
      const LocalState(
        ready: true,
        fingerprint:
            '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
        port: 53317,
        remoteSessions: {
          'session': {
            'status': 'active',
            'role': 'controlled',
            'peer': 'Laptop',
          },
        },
      ),
    );
    for (final label in ['Files', 'Remote', 'Settings', 'Devices']) {
      await tester.tap(find.widgetWithText(NavigationDestination, label));
      await tester.pumpAndSettle();
      expect(find.text('Stop session'), findsOneWidget);
      expect(find.text('Stop all'), findsOneWidget);
      expect(tester.takeException(), isNull);
    }
  });

  testWidgets('file history renders bounded progress and readable sizes', (
    tester,
  ) async {
    await showHome(
      tester,
      const LocalState(
        ready: true,
        transfers: {
          'file': {
            'name': 'example.zip',
            'status': 'complete',
            'direction': 'receive',
            'verifiedBytes': 2048,
            'totalBytes': 1024,
          },
        },
      ),
    );
    await tester.tap(find.widgetWithText(NavigationDestination, 'Files'));
    await tester.pumpAndSettle();
    await tester.scrollUntilVisible(
      find.text('2.0 KB verified of 1.0 KB'),
      150,
      scrollable: find.byType(Scrollable).first,
    );
    await tester.pumpAndSettle();
    expect(find.text('2.0 KB verified of 1.0 KB'), findsOneWidget);
    final progress = tester.widget<LinearProgressIndicator>(
      find.byType(LinearProgressIndicator),
    );
    expect(progress.value, 1);
    expect(find.text('Integrity passport'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
