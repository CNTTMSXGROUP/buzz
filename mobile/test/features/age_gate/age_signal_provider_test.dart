import 'package:buzz/features/age_gate/age_signal_provider.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, null);
  });

  Future<bool> requestWithResponse(Object? response) async {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, (call) async {
          expect(call.method, 'requestAgeSignal');
          expect(call.arguments, isNull);
          return response;
        });
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(ageSignalProvider.notifier).request();
    return container.read(ageSignalProvider);
  }

  test('blocks when the signal upper bound is 17', () async {
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': 17}),
      isTrue,
    );
  });

  test('allows when the signal upper bound is 18', () async {
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': 18}),
      isFalse,
    );
  });

  test('allows when a signal has an open-ended upper bound', () async {
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': null}),
      isFalse,
    );
  });

  test('allows when no signal is available', () async {
    expect(
      await requestWithResponse({'status': 'noSignal', 'ageUpper': null}),
      isFalse,
    );
  });

  test('allows when the platform request throws', () async {
    var requests = 0;
    var delays = 0;
    final provider = NotifierProvider<AgeSignalNotifier, bool>(
      () => AgeSignalNotifier(
        requestSignal: () async {
          requests += 1;
          throw PlatformException(code: 'unavailable');
        },
        delay: (duration) async {
          expect(duration, ageSignalRetryDelay);
          delays += 1;
        },
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();
    await container.read(provider.notifier).request();

    expect(container.read(provider), isFalse);
    expect(requests, 2);
    expect(delays, 1);
  });

  test('retries a transient platform failure and applies the signal', () async {
    var requests = 0;
    final provider = NotifierProvider<AgeSignalNotifier, bool>(
      () => AgeSignalNotifier(
        requestSignal: () async {
          requests += 1;
          if (requests == 1) {
            throw PlatformException(code: 'age_signal_unavailable');
          }
          return {'status': 'signal', 'ageUpper': 17};
        },
        delay: (duration) async {
          expect(duration, ageSignalRetryDelay);
        },
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();

    expect(container.read(provider), isTrue);
    expect(requests, 2);
  });

  test('rejects malformed and unexpected platform responses', () async {
    await expectLater(
      requestWithResponse({'status': 'unknown', 'ageUpper': null}),
      throwsA(isA<StateError>()),
    );
    await expectLater(
      requestWithResponse({'status': 'signal', 'ageUpper': '17'}),
      throwsA(isA<StateError>()),
    );
    await expectLater(
      requestWithResponse({'status': 'signal', 'ageUpper': 17, 'ageLower': 13}),
      throwsA(isA<StateError>()),
    );
    await expectLater(
      requestWithResponse({'status': 'noSignal', 'ageUpper': 17}),
      throwsA(isA<StateError>()),
    );
    expect(await requestWithResponse(null), isFalse);
  });

  test('requests the signal at most once', () async {
    var requests = 0;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, (_) async {
          requests += 1;
          return {'status': 'noSignal', 'ageUpper': null};
        });
    final container = ProviderContainer();
    addTearDown(container.dispose);
    final notifier = container.read(ageSignalProvider.notifier);

    await notifier.request();
    await notifier.request();

    expect(requests, 1);
  });
}
