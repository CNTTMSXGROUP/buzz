import 'dart:async';

import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const ageSignalChannel = MethodChannel('buzz/age_signal');

/// Delay before the single retry of a failed native age-signal request.
const ageSignalRetryDelay = Duration(seconds: 1);

/// Invokes the native age-signal request.
typedef AgeSignalRequest = Future<Map<Object?, Object?>?> Function();

/// Waits before retrying a failed native age-signal request.
typedef AgeSignalDelay = Future<void> Function(Duration duration);

Future<Map<Object?, Object?>?> _requestPlatformAgeSignal() =>
    ageSignalChannel.invokeMapMethod<Object?, Object?>('requestAgeSignal');

Future<void> _delayAgeSignalRetry(Duration duration) =>
    Future<void>.delayed(duration);

bool shouldBlockForAgeSignal(Map<Object?, Object?> response) {
  if (response.length != 2 ||
      !response.containsKey('status') ||
      !response.containsKey('ageUpper')) {
    throw StateError('Unexpected age signal response.');
  }

  final status = response['status'];
  if (status == 'noSignal') {
    if (response['ageUpper'] != null) {
      throw StateError('Unexpected age signal upper bound.');
    }
    return false;
  }
  if (status != 'signal') {
    throw StateError('Unexpected age signal status.');
  }

  final ageUpper = response['ageUpper'];
  if (ageUpper == null) {
    return false;
  }
  if (ageUpper is! int) {
    throw StateError('Unexpected age signal upper bound.');
  }
  return ageUpper < 18;
}

class AgeSignalNotifier extends Notifier<bool> {
  /// Creates an age-signal notifier, optionally with test request hooks.
  AgeSignalNotifier({AgeSignalRequest? requestSignal, AgeSignalDelay? delay})
    : _requestSignal = requestSignal ?? _requestPlatformAgeSignal,
      _delay = delay ?? _delayAgeSignalRetry;

  static const _maxAttempts = 2;

  final AgeSignalRequest _requestSignal;
  final AgeSignalDelay _delay;
  bool _completed = false;
  Future<void>? _requestInFlight;

  @override
  bool build() => false;

  Future<void> request() async {
    if (_completed) return;

    final requestInFlight = _requestInFlight;
    if (requestInFlight != null) {
      await requestInFlight;
      return;
    }

    final request = _requestWithRetry();
    _requestInFlight = request;
    try {
      await request;
    } finally {
      if (identical(_requestInFlight, request)) {
        _requestInFlight = null;
      }
    }
  }

  Future<void> _requestWithRetry() async {
    for (var attempt = 0; attempt < _maxAttempts; attempt += 1) {
      final Map<Object?, Object?>? response;
      try {
        response = await _requestSignal();
      } on MissingPluginException {
        _completed = true;
        state = false;
        return;
      } on PlatformException {
        if (attempt + 1 < _maxAttempts) {
          await _delay(ageSignalRetryDelay);
          continue;
        }
        _completed = true;
        state = false;
        return;
      }

      _completed = true;
      if (response != null) {
        state = shouldBlockForAgeSignal(response);
      }
      return;
    }
  }
}

final ageSignalProvider = NotifierProvider<AgeSignalNotifier, bool>(
  AgeSignalNotifier.new,
);
