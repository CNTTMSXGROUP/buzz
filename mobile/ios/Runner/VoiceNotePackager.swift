import AVFoundation
import Flutter

enum VoiceNotePackager {
    static func package(
        sourcePath: String,
        result: @escaping FlutterResult
    ) {
        let sourceAsset = AVURLAsset(url: URL(fileURLWithPath: sourcePath))
        guard let sourceAudio = sourceAsset.tracks(withMediaType: .audio).first else {
            result(
                FlutterError(
                    code: "transcode_failed",
                    message: "The recording does not contain an audio track.",
                    details: nil
                )
            )
            return
        }

        let duration = sourceAudio.timeRange.duration
        guard duration.isValid, duration.isNumeric, CMTimeCompare(duration, .zero) > 0 else {
            result(
                FlutterError(
                    code: "transcode_failed",
                    message: "The recording has no playable audio.",
                    details: nil
                )
            )
            return
        }

        Self.makeVoiceNoteVideoTrack(duration: duration) { trackResult in
            switch trackResult {
            case let .failure(error):
                result(
                    FlutterError(
                        code: "transcode_failed",
                        message: "Unable to prepare voice note for upload.",
                        details: error.localizedDescription
                    )
                )
            case let .success(videoURL):
                exportVoiceNoteEnvelope(
                    sourceURL: URL(fileURLWithPath: sourcePath),
                    duration: duration,
                    videoURL: videoURL,
                    result: result
                )
            }
        }
    }

    private static func makeVoiceNoteVideoTrack(
        duration: CMTime,
        completion: @escaping (Result<URL, Error>) -> Void
    ) {
        let outputURL = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
            .appendingPathExtension("mp4")

        do {
            let writer = try AVAssetWriter(outputURL: outputURL, fileType: .mp4)
            let input = AVAssetWriterInput(
                mediaType: .video,
                outputSettings: [
                    AVVideoCodecKey: AVVideoCodecType.h264,
                    AVVideoWidthKey: 16,
                    AVVideoHeightKey: 16,
                    AVVideoCompressionPropertiesKey: [
                        AVVideoAverageBitRateKey: 8000,
                        AVVideoExpectedSourceFrameRateKey: 1,
                        AVVideoMaxKeyFrameIntervalKey: 1,
                    ],
                ]
            )
            input.expectsMediaDataInRealTime = false
            guard writer.canAdd(input) else {
                throw NSError(
                    domain: "BuzzVoiceNote",
                    code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "Unable to create the video envelope."]
                )
            }
            writer.add(input)
            let adaptor = AVAssetWriterInputPixelBufferAdaptor(
                assetWriterInput: input,
                sourcePixelBufferAttributes: [
                    kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
                    kCVPixelBufferWidthKey as String: 16,
                    kCVPixelBufferHeightKey as String: 16,
                ]
            )
            guard writer.startWriting() else {
                throw writer.error
                    ?? NSError(
                        domain: "BuzzVoiceNote",
                        code: 2,
                        userInfo: [NSLocalizedDescriptionKey: "Unable to start the video envelope."]
                    )
            }
            writer.startSession(atSourceTime: .zero)

            let queue = DispatchQueue(label: "xyz.block.buzz.voice-note-envelope")
            var appendedFrames = false
            input.requestMediaDataWhenReady(on: queue) {
                guard !appendedFrames, input.isReadyForMoreMediaData else { return }
                appendedFrames = true
                guard
                    let pool = adaptor.pixelBufferPool,
                    let buffer = Self.makeBlackPixelBuffer(pool: pool),
                    adaptor.append(buffer, withPresentationTime: .zero),
                    adaptor.append(buffer, withPresentationTime: duration)
                else {
                    writer.cancelWriting()
                    try? FileManager.default.removeItem(at: outputURL)
                    completion(
                        .failure(
                            writer.error
                                ?? NSError(
                                    domain: "BuzzVoiceNote",
                                    code: 3,
                                    userInfo: [NSLocalizedDescriptionKey: "Unable to write the video envelope."]
                                )
                        )
                    )
                    return
                }
                input.markAsFinished()
                writer.endSession(atSourceTime: duration)
                writer.finishWriting {
                    if writer.status == .completed {
                        completion(.success(outputURL))
                    } else {
                        try? FileManager.default.removeItem(at: outputURL)
                        completion(
                            .failure(
                                writer.error
                                    ?? NSError(
                                        domain: "BuzzVoiceNote",
                                        code: 4,
                                        userInfo: [NSLocalizedDescriptionKey: "Unable to finish the video envelope."]
                                    )
                            )
                        )
                    }
                }
            }
        } catch {
            try? FileManager.default.removeItem(at: outputURL)
            completion(.failure(error))
        }
    }

    private static func makeBlackPixelBuffer(pool: CVPixelBufferPool) -> CVPixelBuffer? {
        var pixelBuffer: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &pixelBuffer) == kCVReturnSuccess,
              let pixelBuffer
        else {
            return nil
        }
        CVPixelBufferLockBaseAddress(pixelBuffer, [])
        if let baseAddress = CVPixelBufferGetBaseAddress(pixelBuffer) {
            memset(baseAddress, 0, CVPixelBufferGetDataSize(pixelBuffer))
        }
        CVPixelBufferUnlockBaseAddress(pixelBuffer, [])
        return pixelBuffer
    }

    private static func exportVoiceNoteEnvelope(
        sourceURL: URL,
        duration: CMTime,
        videoURL: URL,
        result: @escaping FlutterResult
    ) {
        // Reload the recording here so its AVAsset stays alive for the entire
        // composition insert. Keeping only an AVAssetTrack across the asynchronous
        // video-envelope write can leave the track detached from its source asset
        // on physical devices.
        let sourceAsset = AVURLAsset(url: sourceURL)
        let videoAsset = AVURLAsset(url: videoURL)
        let composition = AVMutableComposition()
        do {
            guard
                let sourceAudio = sourceAsset.tracks(withMediaType: .audio).first,
                let sourceVideo = videoAsset.tracks(withMediaType: .video).first,
                let destinationVideo = composition.addMutableTrack(
                    withMediaType: .video,
                    preferredTrackID: kCMPersistentTrackID_Invalid
                ),
                let destinationAudio = composition.addMutableTrack(
                    withMediaType: .audio,
                    preferredTrackID: kCMPersistentTrackID_Invalid
                )
            else {
                throw NSError(
                    domain: "BuzzVoiceNote",
                    code: 5,
                    userInfo: [NSLocalizedDescriptionKey: "Unable to assemble the voice note envelope."]
                )
            }
            let sourceVideoRange = sourceVideo.timeRange
            try destinationVideo.insertTimeRange(sourceVideoRange, of: sourceVideo, at: .zero)
            destinationVideo.scaleTimeRange(
                CMTimeRange(start: .zero, duration: sourceVideoRange.duration),
                toDuration: duration
            )
            try destinationAudio.insertTimeRange(sourceAudio.timeRange, of: sourceAudio, at: .zero)
        } catch {
            try? FileManager.default.removeItem(at: videoURL)
            result(
                FlutterError(
                    code: "transcode_failed",
                    message: "Unable to assemble voice note for upload.",
                    details: error.localizedDescription
                )
            )
            return
        }

        guard let exportSession = AVAssetExportSession(
            asset: composition,
            presetName: AVAssetExportPresetMediumQuality
        ) else {
            try? FileManager.default.removeItem(at: videoURL)
            result(
                FlutterError(
                    code: "transcode_failed",
                    message: "Unable to create voice note export session.",
                    details: nil
                )
            )
            return
        }
        let outputURL = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
            .appendingPathExtension("mp4")
        exportSession.outputURL = outputURL
        exportSession.outputFileType = .mp4
        exportSession.shouldOptimizeForNetworkUse = true
        exportSession.metadata = []
        exportSession.metadataItemFilter = nil
        exportSession.exportAsynchronously {
            try? FileManager.default.removeItem(at: videoURL)
            switch exportSession.status {
            case .completed:
                do {
                    try Self.neutralizeSampleDependencyBoxes(at: outputURL)
                    result(outputURL.path)
                } catch {
                    try? FileManager.default.removeItem(at: outputURL)
                    result(
                        FlutterError(
                            code: "transcode_failed",
                            message: "Unable to canonicalize voice note.",
                            details: error.localizedDescription
                        )
                    )
                }
            default:
                try? FileManager.default.removeItem(at: outputURL)
                result(
                    FlutterError(
                        code: "transcode_failed",
                        message: "Voice note packaging failed.",
                        details: exportSession.error?.localizedDescription
                    )
                )
            }
        }
    }
}
