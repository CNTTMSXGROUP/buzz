import 'dart:async';

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

  Future<AgeSignalState> requestWithResponse(Object? response) async {
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
      AgeSignalState.restricted,
    );
  });

  test('allows when the signal upper bound is 18', () async {
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': 18}),
      AgeSignalState.allowed,
    );
  });

  test('allows when a signal has an open-ended upper bound', () async {
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': null}),
      AgeSignalState.allowed,
    );
  });

  test('allows when no signal is available', () async {
    expect(
      await requestWithResponse({'status': 'noSignal', 'ageUpper': null}),
      AgeSignalState.allowed,
    );
  });

  test('keeps exhausted platform failures gated and retryable', () async {
    var requests = 0;
    var delays = 0;
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(
        requestSignal: () async {
          requests += 1;
          if (requests <= 2) {
            throw PlatformException(code: 'unavailable');
          }
          return {'status': 'noSignal', 'ageUpper': null};
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

    expect(container.read(provider), AgeSignalState.checking);
    expect(requests, 2);
    expect(delays, 1);

    await container.read(provider.notifier).request();

    expect(container.read(provider), AgeSignalState.allowed);
    expect(requests, 3);
    expect(delays, 1);
  });

  test('retries a transient platform failure and applies the signal', () async {
    var requests = 0;
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
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

    expect(container.read(provider), AgeSignalState.restricted);
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
    expect(await requestWithResponse(null), AgeSignalState.allowed);
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

  test('remains checking until the platform request completes', () async {
    final response = Completer<Map<Object?, Object?>?>();
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(requestSignal: () => response.future),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    final request = container.read(provider.notifier).request();

    expect(container.read(provider), AgeSignalState.checking);
    response.complete({'status': 'noSignal', 'ageUpper': null});
    await request;
    expect(container.read(provider), AgeSignalState.allowed);
  });
}
