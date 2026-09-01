import 'dart:async';

import 'package:buzz/app.dart';
import 'package:buzz/features/age_gate/age_restriction_page.dart';
import 'package:buzz/features/age_gate/age_signal_provider.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, null);
  });

  testWidgets('blocks authenticated app content', (tester) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
          ageSignalProvider.overrideWith(() => _BlockingAgeSignalNotifier()),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: const App(),
      ),
    );
    await tester.pump();

    expect(find.byType(AgeRestrictionPage), findsOneWidget);
    expect(find.byType(HomePage), findsNothing);
  });

  testWidgets('keeps app content unmounted until the signal resolves', (
    tester,
  ) async {
    final response = Completer<Object?>();
    final relaySession = _CountingRelaySessionNotifier();
    var requests = 0;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, (call) {
          requests += 1;
          return response.future;
        });
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
          relaySessionProvider.overrideWith(() => relaySession),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: const App(),
      ),
    );

    expect(requests, 1);
    expect(find.bySemanticsLabel('Checking age eligibility'), findsOneWidget);
    expect(find.byType(HomePage), findsNothing);
    expect(find.byType(Navigator), findsNothing);
    expect(relaySession.builds, 0);

    response.complete({'status': 'noSignal', 'ageUpper': null});
    await tester.pump();
    await tester.pump();

    expect(find.bySemanticsLabel('Checking age eligibility'), findsNothing);
    expect(find.byType(HomePage), findsOneWidget);
    expect(find.byType(Navigator), findsOneWidget);
    expect(relaySession.builds, 1);
    expect(requests, 1);
  });
}

class _AuthenticatedAuthNotifier extends AuthNotifier {
  @override
  Future<AuthState> build() async {
    return const AuthState(status: AuthStatus.authenticated);
  }
}

class _BlockingAgeSignalNotifier extends AgeSignalNotifier {
  @override
  AgeSignalState build() => AgeSignalState.restricted;
}

class _CountingRelaySessionNotifier extends RelaySessionNotifier {
  int builds = 0;

  @override
  SessionState build() {
    builds += 1;
    return const SessionState(status: SessionStatus.disconnected);
  }
}
