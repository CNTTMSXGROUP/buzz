import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community_provider.dart';
import '../../shared/push/push_bootstrap.dart';
import '../../shared/push/push_bridge.dart';
import 'age_signal_provider.dart';

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

    useEffect(() {
      unawaited(
        state == AgeSignalState.allowed ? resumeSnapshot() : suspendSnapshot(),
      );
      return null;
    }, [state, suspendSnapshot, resumeSnapshot]);

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
