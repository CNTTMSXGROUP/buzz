import 'dart:async';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:image_picker/image_picker.dart';
import 'package:just_audio/just_audio.dart' as audio;
import 'package:path_provider/path_provider.dart';
import 'package:record/record.dart';

import '../../shared/relay/media_image.dart';

const voiceNoteMaxDuration = Duration(minutes: 5);
const voiceNotePlaybackRates = <double>[1, 1.5, 2, 0.5];
final voiceNoteRouteObserver = RouteObserver<ModalRoute<void>>();

double nextVoiceNotePlaybackRate(double current) {
  final index = voiceNotePlaybackRates.indexOf(current);
  return voiceNotePlaybackRates[(index + 1) % voiceNotePlaybackRates.length];
}

String formatVoiceNotePlaybackRate(double rate) =>
    '${rate == 0.5 ? '.5' : rate.toStringAsFixed(rate % 1 == 0 ? 0 : 1)}×';

@immutable
class VoiceNoteRecording {
  const VoiceNoteRecording({
    required this.file,
    required this.duration,
    required this.waveform,
  });

  final XFile file;
  final Duration duration;
  final List<double> waveform;
}

abstract interface class VoiceNoteRecorder {
  Stream<double> get levels;

  Future<void> start();

  Future<VoiceNoteRecording> stop();

  Future<void> cancel();

  Future<void> dispose();
}

final voiceNoteRecorderFactoryProvider = Provider<VoiceNoteRecorder Function()>(
  (ref) => DeviceVoiceNoteRecorder.new,
);

abstract interface class VoiceNoteRecorderBackend {
  Future<bool> hasPermission();

  Future<void> start(RecordConfig config, {required String path});

  Stream<Amplitude> onAmplitudeChanged(Duration interval);

  Future<String?> stop();

  Future<void> cancel();

  Future<void> dispose();
}

class _DeviceVoiceNoteRecorderBackend implements VoiceNoteRecorderBackend {
  final AudioRecorder _recorder = AudioRecorder();

  @override
  Future<bool> hasPermission() => _recorder.hasPermission();

  @override
  Future<void> start(RecordConfig config, {required String path}) =>
      _recorder.start(config, path: path);

  @override
  Stream<Amplitude> onAmplitudeChanged(Duration interval) =>
      _recorder.onAmplitudeChanged(interval);

  @override
  Future<String?> stop() => _recorder.stop();

  @override
  Future<void> cancel() => _recorder.cancel();

  @override
  Future<void> dispose() => _recorder.dispose();
}

class DeviceVoiceNoteRecorder implements VoiceNoteRecorder {
  DeviceVoiceNoteRecorder({
    VoiceNoteRecorderBackend? backend,
    Future<Directory> Function()? temporaryDirectory,
  }) : _recorder = backend ?? _DeviceVoiceNoteRecorderBackend(),
       _temporaryDirectory = temporaryDirectory ?? getTemporaryDirectory;

  final VoiceNoteRecorderBackend _recorder;
  final Future<Directory> Function() _temporaryDirectory;
  final StreamController<double> _levels = StreamController.broadcast();
  final List<double> _samples = [];
  StreamSubscription<Amplitude>? _amplitudeSubscription;
  Future<void>? _startup;
  Future<void>? _terminalOperation;
  DateTime? _startedAt;
  String? _path;
  int _lifecycleGeneration = 0;
  bool _nativeStarted = false;
  bool _nativeEnded = false;
  bool _finished = false;
  bool _disposed = false;

  @override
  Stream<double> get levels => _levels.stream;

  void _ensureStartupActive(int generation) {
    if (_disposed || _finished || generation != _lifecycleGeneration) {
      throw StateError('Voice note recording was cancelled.');
    }
  }

  @override
  Future<void> start() {
    if (_startup != null || _nativeStarted || _finished || _disposed) {
      return Future.error(
        StateError('Voice note recording cannot be started.'),
      );
    }
    final generation = ++_lifecycleGeneration;
    final startup = _start(generation);
    _startup = startup;
    return startup.whenComplete(() {
      if (identical(_startup, startup)) _startup = null;
    });
  }

  Future<void> _start(int generation) async {
    final hasPermission = await _recorder.hasPermission();
    _ensureStartupActive(generation);
    if (!hasPermission) {
      throw StateError('Microphone access is required to record a voice note.');
    }
    final directory = await _temporaryDirectory();
    _ensureStartupActive(generation);
    final path =
        '${directory.path}${Platform.pathSeparator}'
        'voice-note-${DateTime.now().millisecondsSinceEpoch}.m4a';
    await _recorder.start(
      const RecordConfig(
        encoder: AudioEncoder.aacLc,
        bitRate: 96000,
        sampleRate: 44100,
        numChannels: 1,
        autoGain: true,
        echoCancel: true,
        noiseSuppress: true,
      ),
      path: path,
    );
    _nativeStarted = true;
    _ensureStartupActive(generation);
    _path = path;
    _startedAt = DateTime.now();
    _ensureStartupActive(generation);
    _amplitudeSubscription = _recorder
        .onAmplitudeChanged(const Duration(milliseconds: 80))
        .listen((amplitude) {
          final normalized =
              (math
                          .pow(10, amplitude.current.clamp(-60.0, 0.0) / 20)
                          .toDouble() *
                      4)
                  .clamp(0.04, 1.0);
          _samples.add(normalized);
          if (!_levels.isClosed) _levels.add(normalized);
        });
  }

  @override
  Future<VoiceNoteRecording> stop() {
    if (_finished || !_nativeStarted || _nativeEnded) {
      return Future.error(StateError('Voice note recording is not active.'));
    }
    _finished = true;
    _lifecycleGeneration += 1;
    final operation = _stop();
    final terminalOperation = operation.then<void>((_) {}, onError: (_, _) {});
    _terminalOperation = terminalOperation;
    return operation.whenComplete(() {
      if (identical(_terminalOperation, terminalOperation)) {
        _terminalOperation = null;
      }
    });
  }

  Future<VoiceNoteRecording> _stop() async {
    await _amplitudeSubscription?.cancel();
    final recordedPath = await _recorder.stop() ?? _path;
    _nativeEnded = true;
    if (recordedPath == null || recordedPath.isEmpty) {
      throw StateError('Buzz could not finish the voice note.');
    }
    final startedAt = _startedAt;
    final duration = startedAt == null
        ? Duration.zero
        : DateTime.now().difference(startedAt);
    return VoiceNoteRecording(
      file: XFile(recordedPath, mimeType: 'audio/mp4'),
      duration: duration,
      waveform: List.unmodifiable(_samples),
    );
  }

  @override
  Future<void> cancel() {
    final activeTerminalOperation = _terminalOperation;
    if (activeTerminalOperation != null) return activeTerminalOperation;
    if (_finished) return Future.value();
    _finished = true;
    _lifecycleGeneration += 1;
    final operation = _cancel();
    _terminalOperation = operation;
    return operation.whenComplete(() {
      if (identical(_terminalOperation, operation)) _terminalOperation = null;
    });
  }

  Future<void> _cancel() async {
    try {
      await _startup;
    } catch (_) {
      // A cancellation fence intentionally rejects stale startup work.
    }
    await _amplitudeSubscription?.cancel();
    if (_nativeStarted && !_nativeEnded) {
      await _recorder.cancel();
      _nativeEnded = true;
    }
  }

  @override
  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    _lifecycleGeneration += 1;
    final terminalOperation =
        _terminalOperation ?? (!_finished ? cancel() : null);
    if (terminalOperation != null) {
      try {
        await terminalOperation;
      } catch (_) {
        // Disposal still owns backend release when a terminal operation fails.
      }
    }
    try {
      await _startup;
    } catch (_) {
      // Startup may reject because disposal invalidated its generation.
    }
    await _amplitudeSubscription?.cancel();
    if (_nativeStarted && !_nativeEnded) {
      await _recorder.cancel();
      _nativeEnded = true;
    }
    await _recorder.dispose();
    await _levels.close();
  }
}

@immutable
class VoiceNotePlaybackState {
  const VoiceNotePlaybackState({
    this.position = Duration.zero,
    this.duration = Duration.zero,
    this.isPlaying = false,
    this.isLoading = false,
    this.hasError = false,
  });

  final Duration position;
  final Duration duration;
  final bool isPlaying;
  final bool isLoading;
  final bool hasError;

  VoiceNotePlaybackState copyWith({
    Duration? position,
    Duration? duration,
    bool? isPlaying,
    bool? isLoading,
    bool? hasError,
  }) => VoiceNotePlaybackState(
    position: position ?? this.position,
    duration: duration ?? this.duration,
    isPlaying: isPlaying ?? this.isPlaying,
    isLoading: isLoading ?? this.isLoading,
    hasError: hasError ?? this.hasError,
  );
}

abstract class VoiceNotePlayerController extends ChangeNotifier {
  VoiceNotePlaybackState get state;

  Future<void> loadLocal(String path, {required Duration fallbackDuration});

  Future<void> loadRemote(
    String url, {
    required Map<String, String> headers,
    required Duration fallbackDuration,
  });

  Future<void> toggle();

  Future<void> pause();

  Future<void> seek(Duration position);

  Future<void> setSpeed(double speed);
}

class VoiceNotePlaybackCoordinator {
  VoiceNotePlayerController? _active;

  Future<bool> activate(VoiceNotePlayerController controller) async {
    if (identical(_active, controller)) return true;
    final previous = _active;
    _active = controller;
    await previous?.pause();
    return identical(_active, controller);
  }

  bool ownsPlayback(VoiceNotePlayerController controller) =>
      identical(_active, controller);

  void release(VoiceNotePlayerController controller) {
    if (identical(_active, controller)) _active = null;
  }
}

final voiceNotePlaybackCoordinatorProvider = Provider(
  (ref) => VoiceNotePlaybackCoordinator(),
);

final voiceNotePlayerFactoryProvider =
    Provider<VoiceNotePlayerController Function()>((ref) {
      final coordinator = ref.watch(voiceNotePlaybackCoordinatorProvider);
      final client = ref.watch(mediaHttpClientProvider);
      return () => DeviceVoiceNotePlayerController(
        coordinator: coordinator,
        client: client,
      );
    });

class DeviceVoiceNotePlayerController extends VoiceNotePlayerController {
  DeviceVoiceNotePlayerController({
    required VoiceNotePlaybackCoordinator coordinator,
    required http.Client client,
    Future<Directory> Function()? temporaryDirectory,
    bool? requiresAuthenticatedLocalFile,
  }) : _coordinator = coordinator,
       _client = client,
       _temporaryDirectory = temporaryDirectory ?? getTemporaryDirectory,
       _requiresAuthenticatedLocalFile =
           requiresAuthenticatedLocalFile ?? Platform.isIOS,
       _player = audio.AudioPlayer(useProxyForRequestHeaders: false) {
    _subscriptions.add(
      _player.positionStream.listen((position) {
        _update(_state.copyWith(position: position));
      }),
    );
    _subscriptions.add(
      _player.durationStream.listen((duration) {
        if (duration != null) _update(_state.copyWith(duration: duration));
      }),
    );
    _subscriptions.add(
      _player.playerStateStream.listen((playerState) {
        if (playerState.processingState == audio.ProcessingState.completed) {
          _coordinator.release(this);
          _update(
            _state.copyWith(
              position: Duration.zero,
              isPlaying: false,
              isLoading: false,
            ),
          );
          unawaited(_stopAndRewindCompletedPlayback());
          return;
        }
        _update(
          _state.copyWith(
            isPlaying: playerState.playing,
            isLoading:
                playerState.processingState == audio.ProcessingState.loading ||
                playerState.processingState == audio.ProcessingState.buffering,
          ),
        );
      }),
    );
  }

  final VoiceNotePlaybackCoordinator _coordinator;
  final http.Client _client;
  final Future<Directory> Function() _temporaryDirectory;
  final audio.AudioPlayer _player;
  final bool _requiresAuthenticatedLocalFile;
  final List<StreamSubscription<Object?>> _subscriptions = [];
  VoiceNotePlaybackState _state = const VoiceNotePlaybackState();
  ({String url, Map<String, String> headers, Duration fallbackDuration})?
  _pendingRemote;
  Completer<void>? _downloadAbort;
  File? _downloadingRemoteFile;
  File? _remoteFile;
  bool _disposed = false;

  Future<void> _stopAndRewindCompletedPlayback() async {
    await _player.pause();
    await _player.seek(Duration.zero);
  }

  @override
  VoiceNotePlaybackState get state => _state;

  @override
  Future<void> loadLocal(String path, {required Duration fallbackDuration}) =>
      _load(
        () => _player.setFilePath(path),
        fallbackDuration: fallbackDuration,
      );

  @override
  Future<void> loadRemote(
    String url, {
    required Map<String, String> headers,
    required Duration fallbackDuration,
  }) {
    if (!_requiresAuthenticatedLocalFile || headers.isEmpty) {
      _pendingRemote = null;
      return _load(
        () => _player.setUrl(url, headers: headers),
        fallbackDuration: fallbackDuration,
      );
    }
    _pendingRemote = (
      url: url,
      headers: Map.unmodifiable(headers),
      fallbackDuration: fallbackDuration,
    );
    _update(VoiceNotePlaybackState(duration: fallbackDuration));
    return Future.value();
  }

  Future<Duration?> _loadPendingRemote() async {
    final remote = _pendingRemote;
    if (remote == null) return null;
    final uri = Uri.parse(remote.url);
    final requestAbort = Completer<void>();
    _downloadAbort = requestAbort;
    File? file;
    try {
      final request = http.AbortableStreamedRequest(
        'GET',
        uri,
        abortTrigger: requestAbort.future,
      )..headers.addAll(remote.headers);
      final response = await _client.send(request);
      if (response.statusCode < 200 || response.statusCode >= 300) {
        await response.stream.drain<void>();
        throw HttpException(
          'Voice note download failed (${response.statusCode})',
          uri: uri,
        );
      }
      final directory = await _temporaryDirectory();
      file = File(
        '${directory.path}${Platform.pathSeparator}'
        'buzz-voice-note-${DateTime.now().microsecondsSinceEpoch}.mp4',
      );
      _downloadingRemoteFile = file;
      await response.stream.pipe(file.openWrite());
      if (_disposed || !_coordinator.ownsPlayback(this)) return null;
      await _deleteRemoteFile(_remoteFile);
      _remoteFile = file;
      _downloadingRemoteFile = null;
      _pendingRemote = null;
      return _player.setFilePath(file.path);
    } on http.RequestAbortedException {
      return null;
    } finally {
      if (identical(_downloadAbort, requestAbort)) _downloadAbort = null;
      if (identical(_downloadingRemoteFile, file)) {
        _downloadingRemoteFile = null;
      }
      if (file != null && !identical(_remoteFile, file)) {
        await _deleteRemoteFile(file);
      }
    }
  }

  Future<void> _deleteRemoteFile(File? file) async {
    if (file == null) return;
    try {
      if (await file.exists()) await file.delete();
    } on FileSystemException {
      // Temporary playback cleanup must not make the player fail.
    }
  }

  Future<void> _load(
    Future<Duration?> Function() load, {
    required Duration fallbackDuration,
  }) async {
    _update(
      VoiceNotePlaybackState(duration: fallbackDuration, isLoading: true),
    );
    try {
      final duration = await load();
      _update(
        _state.copyWith(
          duration: duration ?? fallbackDuration,
          isLoading: false,
          hasError: false,
        ),
      );
    } catch (_) {
      _coordinator.release(this);
      _update(_state.copyWith(isLoading: false, hasError: true));
    }
  }

  @override
  Future<void> toggle() async {
    if (_state.hasError || _state.isLoading) return;
    if (_player.playing) {
      await pause();
    } else {
      final ownsPlayback = await _coordinator.activate(this);
      if (!ownsPlayback || _disposed) return;
      final remote = _pendingRemote;
      if (remote != null) {
        await _load(
          _loadPendingRemote,
          fallbackDuration: remote.fallbackDuration,
        );
        if (_pendingRemote != null) return;
      }
      if (_coordinator.ownsPlayback(this) && !_disposed && !_state.hasError) {
        await _player.play();
      }
    }
  }

  @override
  Future<void> pause() async {
    final activeDownloadAbort = _downloadAbort;
    if (activeDownloadAbort != null && !activeDownloadAbort.isCompleted) {
      activeDownloadAbort.complete();
    }
    await _player.pause();
  }

  @override
  Future<void> seek(Duration position) => _player.seek(position);

  @override
  Future<void> setSpeed(double speed) => _player.setSpeed(speed);

  void _update(VoiceNotePlaybackState next) {
    if (_disposed) return;
    _state = next;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _coordinator.release(this);
    final activeDownloadAbort = _downloadAbort;
    if (activeDownloadAbort != null && !activeDownloadAbort.isCompleted) {
      activeDownloadAbort.complete();
    }
    for (final subscription in _subscriptions) {
      unawaited(subscription.cancel());
    }
    unawaited(_player.dispose());
    unawaited(_deleteRemoteFile(_remoteFile));
    super.dispose();
  }
}

String formatVoiceNoteDuration(Duration duration) {
  final totalSeconds = math.max(0, duration.inSeconds);
  final minutes = totalSeconds ~/ 60;
  final seconds = totalSeconds % 60;
  return '$minutes:${seconds.toString().padLeft(2, '0')}';
}

List<double> normalizeVoiceNoteWaveform(
  List<double> samples, {
  int barCount = 36,
}) {
  if (barCount <= 0) return const [];
  if (samples.isEmpty) return List.filled(barCount, 0.12);
  return List.generate(barCount, (index) {
    final start = (index * samples.length / barCount).floor();
    final end = math.max(
      start + 1,
      ((index + 1) * samples.length / barCount).floor(),
    );
    var peak = 0.0;
    for (
      var sampleIndex = start;
      sampleIndex < end && sampleIndex < samples.length;
      sampleIndex++
    ) {
      peak = math.max(peak, samples[sampleIndex]);
    }
    return peak.clamp(0.08, 1.0);
  });
}
