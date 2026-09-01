import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:record/record.dart';
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
    required Map<String, String> headers,
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

      await player.loadRemote(
        'https://example.com/voice-note.mp4',
        headers: const {'Authorization': 'Nostr signed-event'},
        fallbackDuration: const Duration(seconds: 7),
      );
      expect(player.state.isLoading, isFalse);
      expect(player.state.duration, const Duration(seconds: 7));
      expect(client.sent.isCompleted, isFalse);

      final playback = player.toggle();
      final request = await client.sent.future;
      expect(request, isA<http.AbortableStreamedRequest>());
      expect(request.headers['Authorization'], 'Nostr signed-event');

      final abortable = request as http.AbortableStreamedRequest;
      player.dispose();
      await abortable.abortTrigger;
      client.response.completeError(http.RequestAbortedException(request.url));
      await playback;

      expect(directory.listSync().whereType<File>(), isEmpty);
    },
  );

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
