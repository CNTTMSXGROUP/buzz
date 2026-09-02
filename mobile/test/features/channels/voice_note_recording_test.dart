import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:just_audio/just_audio.dart' as audio;
import 'package:record/record.dart';
import 'package:buzz/features/channels/voice_note_composer_recorder.dart';
import 'package:buzz/features/channels/voice_note_recording.dart';

class _DelayedRecorderBackend implements VoiceNoteRecorderBackend {
  final permission = Completer<bool>();
  final nativeStart = Completer<void>();
  final nativeStop = Completer<String?>();
  final amplitudes = StreamController<Amplitude>.broadcast();
  bool startCalled = false;
  bool stopCalled = false;
  bool stopCompleted = false;
  bool cancelCalled = false;
  bool disposeCalled = false;
  bool terminalOverlap = false;

  @override
  Future<bool> hasPermission() => permission.future;

  @override
  Future<void> start(RecordConfig config, {required String path}) {
    startCalled = true;
    return nativeStart.future;
  }

  @override
  Stream<Amplitude> onAmplitudeChanged(Duration interval) => amplitudes.stream;

  @override
  Future<String?> stop() async {
    stopCalled = true;
    final path = await nativeStop.future;
    stopCompleted = true;
    return path;
  }

  @override
  Future<void> cancel() async {
    if (stopCalled && !stopCompleted) terminalOverlap = true;
    cancelCalled = true;
  }

  @override
  Future<void> dispose() async {
    if (stopCalled && !stopCompleted) terminalOverlap = true;
    disposeCalled = true;
    await amplitudes.close();
  }
}

class _DelayedHttpClient extends http.BaseClient {
  final sent = Completer<http.BaseRequest>();
  final response = Completer<http.StreamedResponse>();

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) {
    sent.complete(request);
    return response.future;
  }
}

class _SequencedHttpClient extends http.BaseClient {
  final requests = <http.BaseRequest>[];
  final responses = <Completer<http.StreamedResponse>>[];

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) {
    requests.add(request);
    final response = Completer<http.StreamedResponse>();
    responses.add(response);
    return response.future;
  }
}

class _FakeAudioPlayerBackend implements VoiceNoteAudioPlayerBackend {
  final positions = StreamController<Duration>.broadcast();
  final durations = StreamController<Duration?>.broadcast();
  final states = StreamController<audio.PlayerState>.broadcast();
  final pathLoads = <Completer<Duration?>>[];
  final urlLoads = <Completer<Duration?>>[];
  bool delayPathLoads = false;
  bool delayUrlLoads = false;
  bool delayPlay = false;
  bool _playing = false;
  int playCount = 0;
  int pauseCount = 0;
  final loadedPaths = <String>[];
  final loadedUrls = <String>[];

  @override
  Stream<Duration> get positionStream => positions.stream;

  @override
  Stream<Duration?> get durationStream => durations.stream;

  @override
  Stream<audio.PlayerState> get playerStateStream => states.stream;

  @override
  bool get playing => _playing;

  @override
  Future<Duration?> setFilePath(String path) {
    loadedPaths.add(path);
    if (!delayPathLoads) return Future.value(const Duration(seconds: 7));
    final load = Completer<Duration?>();
    pathLoads.add(load);
    return load.future.whenComplete(() => pathLoads.remove(load));
  }

  @override
  Future<Duration?> setUrl(String url, {Map<String, String>? headers}) {
    loadedUrls.add(url);
    if (!delayUrlLoads) return Future.value(const Duration(seconds: 7));
    final load = Completer<Duration?>();
    urlLoads.add(load);
    return load.future.whenComplete(() => urlLoads.remove(load));
  }

  @override
  Future<void> play() {
    _playing = true;
    playCount += 1;
    if (!delayPlay) return Future.value();
    final playback = Completer<void>();
    late final StreamSubscription<audio.PlayerState> subscription;
    subscription = states.stream.listen((state) {
      if (!state.playing && !playback.isCompleted) playback.complete();
    });
    return playback.future.whenComplete(subscription.cancel);
  }

  @override
  Future<void> pause() async {
    pauseCount += 1;
    _playing = false;
    states.add(audio.PlayerState(false, audio.ProcessingState.ready));
  }

  @override
  Future<void> seek(Duration position) async {}

  @override
  Future<void> setSpeed(double speed) async {}

  @override
  Future<void> dispose() async {
    await positions.close();
    await durations.close();
    await states.close();
  }
}

class _CoordinatedPlayer extends VoiceNotePlayerController {
  _CoordinatedPlayer(
    this.coordinator, {
    required this.source,
    required this.isRemote,
  });

  final VoiceNotePlaybackCoordinator coordinator;
  final String source;
  final bool isRemote;
  VoiceNotePlaybackState _state = const VoiceNotePlaybackState();
  int pauseCount = 0;

  @override
  VoiceNotePlaybackState get state => _state;

  @override
  Future<void> loadLocal(
    String path, {
    required Duration fallbackDuration,
  }) async {}

  @override
  Future<void> loadRemote(
    String url, {
    required Map<String, String> Function() headers,
    required Duration fallbackDuration,
  }) async {}

  @override
  Future<void> pause() async {
    pauseCount += 1;
    _state = _state.copyWith(isPlaying: false);
  }

  @override
  Future<void> seek(Duration position) async {}

  @override
  Future<void> setSpeed(double speed) async {}

  @override
  Future<void> toggle() async {
    if (_state.isPlaying) {
      await pause();
      return;
    }
    if (await coordinator.activate(this)) {
      _state = _state.copyWith(isPlaying: true);
    }
  }

  void complete() {
    _state = _state.copyWith(isPlaying: false);
    coordinator.release(this);
  }

  @override
  void dispose() {
    coordinator.release(this);
    super.dispose();
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test(
    'dropped finalized recordings are deleted without surfacing errors',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'voice-note-dropped-recording-test',
      );
      addTearDown(() => directory.delete(recursive: true));
      final recording = File('${directory.path}/recording.m4a');
      await recording.writeAsBytes([1, 2, 3]);

      await deleteDroppedVoiceNoteRecording(recording.path);
      await deleteDroppedVoiceNoteRecording(recording.path);

      expect(await recording.exists(), isFalse);
    },
  );

  test('cancellation fences delayed permission before native start', () async {
    final backend = _DelayedRecorderBackend();
    final directory = await Directory.systemTemp.createTemp('voice-note-test');
    addTearDown(() => directory.delete(recursive: true));
    final recorder = DeviceVoiceNoteRecorder(
      backend: backend,
      temporaryDirectory: () async => directory,
    );

    final startup = recorder.start();
    final startupExpectation = expectLater(startup, throwsStateError);
    final cancellation = recorder.cancel();
    backend.permission.complete(true);

    await cancellation;
    await startupExpectation;
    expect(backend.startCalled, isFalse);
    await recorder.dispose();
    expect(backend.disposeCalled, isTrue);
  });

  test('cancellation ends native recording when start resolves late', () async {
    final backend = _DelayedRecorderBackend();
    final directory = await Directory.systemTemp.createTemp('voice-note-test');
    addTearDown(() => directory.delete(recursive: true));
    final recorder = DeviceVoiceNoteRecorder(
      backend: backend,
      temporaryDirectory: () async => directory,
    );

    final startup = recorder.start();
    final startupExpectation = expectLater(startup, throwsStateError);
    backend.permission.complete(true);
    await Future<void>.delayed(Duration.zero);
    expect(backend.startCalled, isTrue);

    final cancellation = recorder.cancel();
    backend.nativeStart.complete();
    await cancellation;
    await startupExpectation;

    expect(backend.cancelCalled, isTrue);
    await recorder.dispose();
  });

  test(
    'dispose waits for an in-flight stop before releasing backend',
    () async {
      final backend = _DelayedRecorderBackend();
      final directory = await Directory.systemTemp.createTemp(
        'voice-note-test',
      );
      addTearDown(() => directory.delete(recursive: true));
      final recorder = DeviceVoiceNoteRecorder(
        backend: backend,
        temporaryDirectory: () async => directory,
      );
      backend.permission.complete(true);
      backend.nativeStart.complete();
      await recorder.start();

      final stopping = recorder.stop();
      final disposing = recorder.dispose();
      await Future<void>.delayed(Duration.zero);

      expect(backend.stopCalled, isTrue);
      expect(backend.cancelCalled, isFalse);
      expect(backend.disposeCalled, isFalse);
      expect(backend.terminalOverlap, isFalse);

      backend.nativeStop.complete('/tmp/voice-note-test.m4a');
      final recording = await stopping;
      await disposing;

      expect(recording.file.path, '/tmp/voice-note-test.m4a');
      expect(backend.stopCompleted, isTrue);
      expect(backend.cancelCalled, isFalse);
      expect(backend.disposeCalled, isTrue);
      expect(backend.terminalOverlap, isFalse);
    },
  );

  test(
    'authenticated iOS playback waits for play and aborts on disposal',
    () async {
      final client = _DelayedHttpClient();
      final directory = await Directory.systemTemp.createTemp(
        'voice-note-playback-test',
      );
      addTearDown(() => directory.delete(recursive: true));
      final player = DeviceVoiceNotePlayerController(
        coordinator: VoiceNotePlaybackCoordinator(),
        client: client,
        temporaryDirectory: () async => directory,
        requiresAuthenticatedLocalFile: true,
      );

      var authGeneration = 0;
      await player.loadRemote(
        'https://example.com/voice-note.mp4',
        headers: () => {
          'Authorization': 'Nostr signed-event-${authGeneration++}',
        },
        fallbackDuration: const Duration(seconds: 7),
      );
      expect(authGeneration, 0);
      expect(player.state.isLoading, isFalse);
      expect(player.state.duration, const Duration(seconds: 7));
      expect(client.sent.isCompleted, isFalse);

      final playback = player.toggle();
      final request = await client.sent.future;
      expect(request, isA<http.AbortableStreamedRequest>());
      expect(request.headers['Authorization'], 'Nostr signed-event-0');
      expect(authGeneration, 1);

      final abortable = request as http.AbortableStreamedRequest;
      player.dispose();
      await abortable.abortTrigger;
      client.response.completeError(http.RequestAbortedException(request.url));
      await playback;

      expect(directory.listSync().whereType<File>(), isEmpty);
    },
  );

  test(
    'second remote toggle cancels download and third retries with fresh auth',
    () async {
      final client = _SequencedHttpClient();
      final audioPlayer = _FakeAudioPlayerBackend();
      final directory = await Directory.systemTemp.createTemp(
        'voice-note-toggle-cancel-test',
      );
      addTearDown(() async {
        if (await directory.exists()) await directory.delete(recursive: true);
      });
      final player = DeviceVoiceNotePlayerController(
        coordinator: VoiceNotePlaybackCoordinator(),
        client: client,
        temporaryDirectory: () async => directory,
        requiresAuthenticatedLocalFile: true,
        player: audioPlayer,
      );
      addTearDown(player.dispose);
      var authGeneration = 0;
      await player.loadRemote(
        'https://example.com/voice-note.mp4',
        headers: () => {
          'Authorization': 'Nostr signed-event-${authGeneration++}',
        },
        fallbackDuration: const Duration(seconds: 7),
      );

      final firstToggle = player.toggle();
      await Future<void>.delayed(Duration.zero);
      final firstRequest =
          client.requests.single as http.AbortableStreamedRequest;
      final secondToggle = player.toggle();
      await firstRequest.abortTrigger;
      client.responses.single.completeError(
        http.RequestAbortedException(firstRequest.url),
      );
      await Future.wait([firstToggle, secondToggle]);

      expect(client.requests, hasLength(1));
      expect(authGeneration, 1);
      expect(audioPlayer.loadedPaths, isEmpty);
      expect(audioPlayer.playCount, 0);
      expect(player.state.isLoading, isFalse);
      expect(directory.listSync().whereType<File>(), isEmpty);

      final retry = player.toggle();
      await Future<void>.delayed(Duration.zero);
      expect(client.requests, hasLength(2));
      expect(
        client.requests.last.headers['Authorization'],
        'Nostr signed-event-1',
      );
      client.responses.last.complete(
        http.StreamedResponse(Stream.value(<int>[1, 2, 3]), 200),
      );
      await retry;

      expect(authGeneration, 2);
      expect(audioPlayer.loadedPaths, hasLength(1));
      expect(audioPlayer.playCount, 1);
    },
  );

  test('transient remote failure retries with fresh auth and plays', () async {
    final client = _SequencedHttpClient();
    final audioPlayer = _FakeAudioPlayerBackend();
    final directory = await Directory.systemTemp.createTemp(
      'voice-note-retry-test',
    );
    addTearDown(() async {
      if (await directory.exists()) await directory.delete(recursive: true);
    });
    final player = DeviceVoiceNotePlayerController(
      coordinator: VoiceNotePlaybackCoordinator(),
      client: client,
      temporaryDirectory: () async => directory,
      requiresAuthenticatedLocalFile: true,
      player: audioPlayer,
    );
    addTearDown(player.dispose);
    var authGeneration = 0;
    await player.loadRemote(
      'https://example.com/voice-note.mp4',
      headers: () => {
        'Authorization': 'Nostr signed-event-${authGeneration++}',
      },
      fallbackDuration: const Duration(seconds: 7),
    );

    final firstToggle = player.toggle();
    await Future<void>.delayed(Duration.zero);
    client.responses.single.complete(
      http.StreamedResponse(Stream<List<int>>.empty(), 503),
    );
    await firstToggle;
    expect(player.state.hasError, isTrue);
    expect(client.requests, hasLength(1));
    expect(audioPlayer.playCount, 0);

    final retry = player.toggle();
    await Future<void>.delayed(Duration.zero);
    expect(client.requests, hasLength(2));
    expect(
      client.requests.last.headers['Authorization'],
      'Nostr signed-event-1',
    );
    client.responses.last.complete(
      http.StreamedResponse(Stream.value(<int>[4, 5, 6]), 200),
    );
    await retry;

    expect(authGeneration, 2);
    expect(player.state.hasError, isFalse);
    expect(audioPlayer.playCount, 1);
  });

  test(
    'pause aborts a rapid-toggle download without temporary residue',
    () async {
      final client = _SequencedHttpClient();
      final audioPlayer = _FakeAudioPlayerBackend();
      final directory = await Directory.systemTemp.createTemp(
        'voice-note-pause-test',
      );
      addTearDown(() => directory.delete(recursive: true));
      final player = DeviceVoiceNotePlayerController(
        coordinator: VoiceNotePlaybackCoordinator(),
        client: client,
        temporaryDirectory: () async => directory,
        requiresAuthenticatedLocalFile: true,
        player: audioPlayer,
      );
      addTearDown(player.dispose);
      await player.loadRemote(
        'https://example.com/voice-note.mp4',
        headers: () => const {},
        fallbackDuration: const Duration(seconds: 7),
      );

      final firstToggle = player.toggle();
      final secondToggle = player.toggle();
      await Future<void>.delayed(Duration.zero);
      final request = client.requests.single as http.AbortableStreamedRequest;
      await player.pause();
      await request.abortTrigger;
      client.responses.single.completeError(
        http.RequestAbortedException(request.url),
      );
      await Future.wait([firstToggle, secondToggle]);

      expect(client.requests, hasLength(1));
      expect(audioPlayer.loadedPaths, isEmpty);
      expect(audioPlayer.playCount, 0);
      expect(directory.listSync().whereType<File>(), isEmpty);
    },
  );

  test(
    'source replacement aborts the owned download without stale playback',
    () async {
      final client = _SequencedHttpClient();
      final audioPlayer = _FakeAudioPlayerBackend()..delayPathLoads = true;
      final directory = await Directory.systemTemp.createTemp(
        'voice-note-source-replacement-test',
      );
      addTearDown(() => directory.delete(recursive: true));
      final player = DeviceVoiceNotePlayerController(
        coordinator: VoiceNotePlaybackCoordinator(),
        client: client,
        temporaryDirectory: () async => directory,
        requiresAuthenticatedLocalFile: true,
        player: audioPlayer,
      );
      addTearDown(player.dispose);
      await player.loadRemote(
        'https://example.com/first.mp4',
        headers: () => const {},
        fallbackDuration: const Duration(seconds: 7),
      );

      final playback = player.toggle();
      await Future<void>.delayed(Duration.zero);
      client.responses.single.complete(
        http.StreamedResponse(Stream.value(<int>[1, 2, 3]), 200),
      );
      while (audioPlayer.pathLoads.isEmpty) {
        await Future<void>.delayed(Duration.zero);
      }
      final staleFile = File(audioPlayer.loadedPaths.single);
      expect(await staleFile.exists(), isTrue);

      final replacement = player.loadLocal(
        '${directory.path}/replacement.m4a',
        fallbackDuration: const Duration(seconds: 8),
      );
      await Future<void>.delayed(Duration.zero);
      audioPlayer.pathLoads.first.complete(const Duration(seconds: 7));
      await playback;
      while (await staleFile.exists()) {
        await Future<void>.delayed(Duration.zero);
      }

      expect(audioPlayer.playCount, 0);
      expect(player.state.duration, const Duration(seconds: 8));
      expect(player.state.isLoading, isTrue);

      audioPlayer.pathLoads.last.complete(const Duration(seconds: 8));
      await replacement;
      expect(player.state.duration, const Duration(seconds: 8));
      expect(player.state.isLoading, isFalse);
    },
  );

  test('pause interrupts production-shaped pending playback', () async {
    final audioPlayer = _FakeAudioPlayerBackend()..delayPlay = true;
    final player = DeviceVoiceNotePlayerController(
      coordinator: VoiceNotePlaybackCoordinator(),
      client: _SequencedHttpClient(),
      requiresAuthenticatedLocalFile: false,
      player: audioPlayer,
    );
    addTearDown(player.dispose);
    await player.loadLocal(
      '/tmp/voice-note.m4a',
      fallbackDuration: const Duration(seconds: 7),
    );

    final playing = player.toggle();
    await Future<void>.delayed(Duration.zero);
    expect(audioPlayer.playCount, 1);
    expect(audioPlayer.playing, isTrue);

    await player.toggle();
    await playing;

    expect(audioPlayer.pauseCount, 1);
    expect(audioPlayer.playing, isFalse);
  });

  test('Android remote failure retains the source for retry', () async {
    final audioPlayer = _FakeAudioPlayerBackend()..delayUrlLoads = true;
    final player = DeviceVoiceNotePlayerController(
      coordinator: VoiceNotePlaybackCoordinator(),
      client: _SequencedHttpClient(),
      requiresAuthenticatedLocalFile: false,
      player: audioPlayer,
    );
    addTearDown(player.dispose);
    var authGeneration = 0;

    final initialLoad = player.loadRemote(
      'https://example.com/voice-note.mp4',
      headers: () => {
        'Authorization': 'Nostr signed-event-${authGeneration++}',
      },
      fallbackDuration: const Duration(seconds: 7),
    );
    await Future<void>.delayed(Duration.zero);
    audioPlayer.urlLoads.single.completeError(StateError('network failed'));
    await initialLoad;

    expect(player.state.hasError, isTrue);
    expect(audioPlayer.loadedUrls, hasLength(1));

    final retry = player.toggle();
    await Future<void>.delayed(Duration.zero);
    expect(audioPlayer.loadedUrls, hasLength(2));
    expect(authGeneration, 2);
    audioPlayer.urlLoads.single.complete(const Duration(seconds: 7));
    await retry;

    expect(player.state.hasError, isFalse);
    expect(audioPlayer.playCount, 1);
  });

  test(
    'playback coordinator arbitrates instances and releases ownership',
    () async {
      final coordinator = VoiceNotePlaybackCoordinator();
      final first = _CoordinatedPlayer(
        coordinator,
        source: 'https://example.com/first.mp4',
        isRemote: true,
      );
      final second = _CoordinatedPlayer(
        coordinator,
        source: 'https://example.com/second.mp4',
        isRemote: true,
      );
      final duplicateSource = _CoordinatedPlayer(
        coordinator,
        source: first.source,
        isRemote: true,
      );
      final composerPreview = _CoordinatedPlayer(
        coordinator,
        source: '/tmp/voice-note.m4a',
        isRemote: false,
      );
      addTearDown(second.dispose);
      addTearDown(duplicateSource.dispose);
      addTearDown(composerPreview.dispose);
      expect(duplicateSource.source, first.source);

      await first.toggle();
      await second.toggle();

      expect(first.pauseCount, 1);
      expect(first.state.isPlaying, isFalse);
      expect(second.state.isPlaying, isTrue);

      await duplicateSource.toggle();
      expect(second.pauseCount, 1);
      expect(second.state.isPlaying, isFalse);
      expect(duplicateSource.state.isPlaying, isTrue);

      duplicateSource.complete();
      await composerPreview.toggle();
      expect(duplicateSource.pauseCount, 0);
      expect(composerPreview.isRemote, isFalse);
      expect(composerPreview.state.isPlaying, isTrue);

      await first.toggle();
      expect(composerPreview.pauseCount, 1);
      expect(composerPreview.state.isPlaying, isFalse);
      expect(first.state.isPlaying, isTrue);

      first.dispose();
      await second.toggle();
      expect(first.pauseCount, 1);
      expect(second.state.isPlaying, isTrue);
    },
  );
}
