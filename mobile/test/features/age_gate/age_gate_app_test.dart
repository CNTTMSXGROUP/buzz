import 'dart:async';

import 'package:buzz/app.dart';
import 'package:buzz/features/age_gate/age_restriction_page.dart';
import 'package:buzz/features/age_gate/age_signal_push_bootstrap.dart';
import 'package:buzz/features/age_gate/age_signal_provider.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, null);
  });

  test('backs off repeated snapshot transition failures', () {
    expect(ageSignalPushSnapshotRetryDelay(0), const Duration(seconds: 5));
    expect(ageSignalPushSnapshotRetryDelay(1), const Duration(seconds: 10));
    expect(ageSignalPushSnapshotRetryDelay(5), const Duration(seconds: 160));
    expect(ageSignalPushSnapshotRetryDelay(6), const Duration(minutes: 5));
    expect(ageSignalPushSnapshotRetryDelay(100), const Duration(minutes: 5));
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
        child: const AgeSignalPushBootstrap(child: App()),
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
    var snapshotSuspensions = 0;
    var snapshotRestorations = 0;
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
          suspendCommunitySnapshotForAgeCheckProvider.overrideWithValue(
            () async {
              snapshotSuspensions += 1;
              if (snapshotSuspensions == 1) {
                throw StateError('injected suspension failure');
              }
            },
          ),
          resumeCommunitySnapshotAfterAgeCheckProvider.overrideWithValue(
            () async {
              snapshotRestorations += 1;
              if (snapshotRestorations == 1) {
                throw StateError('injected restoration failure');
              }
            },
          ),
          ageSignalPushSnapshotRetryWaitProvider.overrideWithValue(
            (_) async {},
          ),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: const AgeSignalPushBootstrap(child: App()),
      ),
    );

    expect(requests, 1);
    expect(find.bySemanticsLabel('Checking age eligibility'), findsOneWidget);
    expect(find.byType(HomePage), findsNothing);
    expect(find.byType(Navigator), findsNothing);
    expect(relaySession.builds, 0);
    await tester.pump();
    await tester.pump();
    expect(snapshotSuspensions, 2);
    expect(snapshotRestorations, 0);

    response.complete({'status': 'noSignal', 'ageUpper': null});
    await tester.pump();
    await tester.pump();

    expect(find.bySemanticsLabel('Checking age eligibility'), findsNothing);
    expect(find.byType(HomePage), findsOneWidget);
    expect(find.byType(Navigator), findsOneWidget);
    expect(relaySession.builds, 1);
    expect(requests, 1);
    await tester.pump();
    await tester.pump();
    expect(snapshotRestorations, 2);
  });

  testWidgets('offers a retry after the native age check fails', (
    tester,
  ) async {
    var requests = 0;
    var snapshotSuspensions = 0;
    var snapshotRestorations = 0;
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
          ageSignalProvider.overrideWith(
            () => AgeSignalNotifier(
              requestSignal: () async {
                requests += 1;
                if (requests <= 2) {
                  throw PlatformException(code: 'unavailable');
                }
                return {'status': 'noSignal', 'ageUpper': null};
              },
              delay: (_) async {},
            ),
          ),
          suspendCommunitySnapshotForAgeCheckProvider.overrideWithValue(
            () async => snapshotSuspensions += 1,
          ),
          resumeCommunitySnapshotAfterAgeCheckProvider.overrideWithValue(
            () async => snapshotRestorations += 1,
          ),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: const AgeSignalPushBootstrap(child: App()),
      ),
    );
    await tester.pump();
    await tester.pump();

    expect(find.text('Try again'), findsOneWidget);
    expect(find.byType(HomePage), findsNothing);
    expect(snapshotSuspensions, greaterThanOrEqualTo(1));
    expect(snapshotRestorations, 0);

    await tester.tap(find.text('Try again'));
    await tester.pump();
    await tester.pump();

    expect(requests, 3);
    expect(find.text('Try again'), findsNothing);
    expect(find.byType(HomePage), findsOneWidget);
    expect(snapshotRestorations, 1);
  });

  testWidgets('reloads failed community storage on resume before cleanup', (
    tester,
  ) async {
    final communities = _RecoveringCommunityListNotifier();

    await tester.pumpWidget(
      ProviderScope(
        retry: (_, _) => null,
        overrides: [
          ageSignalProvider.overrideWith(() => _BlockingAgeSignalNotifier()),
          communityListProvider.overrideWith(() => communities),
        ],
        child: const AgeSignalPushBootstrap(child: SizedBox()),
      ),
    );
    await tester.pump();

    expect(communities.builds, 1);
    expect(communities.cleanups, 0);

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pump();
    await tester.pump();

    expect(communities.builds, 2);
    expect(communities.cleanups, 1);
  });

  testWidgets('retries failed restricted push cleanup without an app resume', (
    tester,
  ) async {
    final communities = _RetryingCleanupCommunityListNotifier();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          ageSignalProvider.overrideWith(() => _BlockingAgeSignalNotifier()),
          communityListProvider.overrideWith(() => communities),
          ageSignalPushSnapshotRetryWaitProvider.overrideWithValue(
            (_) async {},
          ),
        ],
        child: const AgeSignalPushBootstrap(child: SizedBox()),
      ),
    );
    await tester.pump();
    await tester.pump();

    expect(communities.cleanups, 2);
  });

  testWidgets(
    'purges restricted notifications before community storage recovers',
    (tester) async {
      var purges = 0;

      await tester.pumpWidget(
        ProviderScope(
          retry: (_, _) => null,
          overrides: [
            ageSignalProvider.overrideWith(() => _BlockingAgeSignalNotifier()),
            communityListProvider.overrideWith(
              () => _UnavailableCommunityListNotifier(),
            ),
            ageRestrictedNotificationPurgerProvider.overrideWithValue(() async {
              purges += 1;
            }),
          ],
          child: const AgeSignalPushBootstrap(child: SizedBox()),
        ),
      );
      await tester.pump();

      expect(purges, 1);
    },
  );

  testWidgets('retries a failed restricted notification purge', (tester) async {
    var purges = 0;

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          ageSignalProvider.overrideWith(() => _BlockingAgeSignalNotifier()),
          communityListProvider.overrideWith(
            () => _RetryingCleanupCommunityListNotifier(),
          ),
          ageRestrictedNotificationPurgerProvider.overrideWithValue(() async {
            purges += 1;
            if (purges == 1) {
              throw StateError('injected notification purge failure');
            }
          }),
          ageSignalPushSnapshotRetryWaitProvider.overrideWithValue(
            (_) async {},
          ),
        ],
        child: const AgeSignalPushBootstrap(child: SizedBox()),
      ),
    );
    await tester.pump();
    await tester.pump();

    expect(purges, 2);
  });

  testWidgets(
    'retries a successful purge for interactions donated by stale extensions',
    (tester) async {
      var purges = 0;
      VoidCallback? runMaintenance;

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            ageSignalProvider.overrideWith(() => _BlockingAgeSignalNotifier()),
            communityListProvider.overrideWith(
              () => _SuccessfulCleanupCommunityListNotifier(),
            ),
            ageRestrictedNotificationPurgerProvider.overrideWithValue(() async {
              purges += 1;
            }),
            ageRestrictedNotificationMaintenanceScheduleProvider
                .overrideWithValue((callback) {
                  runMaintenance = callback;
                  return () {};
                }),
          ],
          child: const AgeSignalPushBootstrap(child: SizedBox()),
        ),
      );
      await tester.pump();
      expect(purges, 1);

      runMaintenance!();
      await tester.pump();
      await tester.pump();

      expect(purges, 2);
    },
  );
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

  @override
  Future<void> request() async {}
}

class _CountingRelaySessionNotifier extends RelaySessionNotifier {
  int builds = 0;

  @override
  SessionState build() {
    builds += 1;
    return const SessionState(status: SessionStatus.disconnected);
  }
}

class _RecoveringCommunityListNotifier extends CommunityListNotifier {
  int builds = 0;
  int cleanups = 0;

  @override
  Future<List<Community>> build() async {
    builds += 1;
    if (builds == 1) throw StateError('secure storage unavailable');
    return const [];
  }

  @override
  Future<void> enforceAgeRestrictionOnPush() async {
    cleanups += 1;
  }
}

class _RetryingCleanupCommunityListNotifier extends CommunityListNotifier {
  int cleanups = 0;

  @override
  Future<List<Community>> build() async => const [];

  @override
  Future<void> enforceAgeRestrictionOnPush() async {
    cleanups += 1;
    if (cleanups == 1) {
      throw StateError('injected restricted cleanup failure');
    }
  }
}

class _SuccessfulCleanupCommunityListNotifier extends CommunityListNotifier {
  @override
  Future<List<Community>> build() async => const [];

  @override
  Future<void> enforceAgeRestrictionOnPush() async {}
}

class _UnavailableCommunityListNotifier extends CommunityListNotifier {
  @override
  Future<List<Community>> build() async {
    throw StateError('secure storage unavailable');
  }
}
