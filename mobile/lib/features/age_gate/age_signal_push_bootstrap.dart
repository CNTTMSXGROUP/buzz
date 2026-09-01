import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/push/push_bootstrap.dart';
import 'age_signal_provider.dart';

/// Starts the push lifecycle only after the launch age check allows access.
class AgeSignalPushBootstrap extends ConsumerWidget {
  /// Creates the production push boundary around [child].
  const AgeSignalPushBootstrap({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (ref.watch(ageSignalProvider) != AgeSignalState.allowed) return child;
    return BuzzPushBootstrap(child: child);
  }
}
