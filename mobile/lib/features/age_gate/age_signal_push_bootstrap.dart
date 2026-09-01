import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community_provider.dart';
import '../../shared/push/push_bootstrap.dart';
import '../../shared/push/push_bridge.dart';
import 'age_signal_provider.dart';

/// Delay between failed age-gate snapshot transitions.
const ageSignalPushSnapshotRetryDelay = Duration(seconds: 5);

/// Waits before retrying a failed age-gate snapshot transition.
typedef AgeSignalPushSnapshotRetryWait =
    Future<void> Function(Duration duration);

/// Retry wait used by the launch age gate's notification snapshot boundary.
final ageSignalPushSnapshotRetryWaitProvider =
    Provider<AgeSignalPushSnapshotRetryWait>((ref) {
      return Future<void>.delayed;
    });

/// Starts the push lifecycle only after the launch age check allows access.
class AgeSignalPushBootstrap extends HookConsumerWidget {
  /// Creates the production push boundary around [child].
  const AgeSignalPushBootstrap({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(ageSignalProvider);
    final suspendSnapshot = ref.watch(
      suspendCommunitySnapshotForAgeCheckProvider,
    );
    final resumeSnapshot = ref.watch(
      resumeCommunitySnapshotAfterAgeCheckProvider,
    );
    final waitBeforeRetry = ref.watch(ageSignalPushSnapshotRetryWaitProvider);
    final retryGeneration = useState(0);

    useEffect(
      () {
        var cancelled = false;
        unawaited(() async {
          try {
            await (state == AgeSignalState.allowed
                ? resumeSnapshot()
                : suspendSnapshot());
          } catch (_) {
            await waitBeforeRetry(ageSignalPushSnapshotRetryDelay);
            if (!cancelled) retryGeneration.value += 1;
          }
        }());
        return () => cancelled = true;
      },
      [
        state,
        suspendSnapshot,
        resumeSnapshot,
        waitBeforeRetry,
        retryGeneration.value,
      ],
    );

    return switch (state) {
      AgeSignalState.allowed => BuzzPushBootstrap(child: child),
      AgeSignalState.restricted => _AgeRestrictedPushCleanup(child: child),
      AgeSignalState.checking || AgeSignalState.retryableFailure => child,
    };
  }
}

class _AgeRestrictedPushCleanup extends HookConsumerWidget {
  const _AgeRestrictedPushCleanup({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final communitiesReady = ref.watch(communityListProvider).hasValue;
    final resumeGeneration = useState(0);

    useEffect(() {
      final listener = AppLifecycleListener(
        onResume: () {
          if (ref.read(communityListProvider).hasError) {
            ref.invalidate(communityListProvider);
          }
          resumeGeneration.value += 1;
        },
      );
      return listener.dispose;
    }, const []);

    useEffect(() {
      if (communitiesReady) {
        unawaited(
          ref
              .read(communityListProvider.notifier)
              .enforceAgeRestrictionOnPush()
              .catchError((Object error, StackTrace stackTrace) {
                reportPushLeaseCleanupError(error, stackTrace);
              }),
        );
      }
      return null;
    }, [communitiesReady, resumeGeneration.value]);

    return child;
  }
}
