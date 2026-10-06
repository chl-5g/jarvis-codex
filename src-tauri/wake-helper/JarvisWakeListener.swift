import AppKit
import AVFoundation
import Foundation
import Speech

final class WakeListener {
    private let recognizer = SFSpeechRecognizer(locale: Locale(identifier: "zh-CN"))
    private let audioEngine = AVAudioEngine()
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var hasWoken = false
    private let eventFile: URL?
    private let hostApp: URL?

    private let phrases: [String]

    init() {
        if let url = Bundle.main.url(forResource: "wake", withExtension: "json"),
           let data = try? Data(contentsOf: url),
           let config = try? JSONDecoder().decode(WakeConfig.self, from: data) {
            phrases = config.phrases
        } else {
            phrases = []
        }
        if
            let index = CommandLine.arguments.firstIndex(of: "--event-file"),
            CommandLine.arguments.indices.contains(index + 1)
        {
            eventFile = URL(fileURLWithPath: CommandLine.arguments[index + 1])
        } else {
            eventFile = nil
        }
        if
            let index = CommandLine.arguments.firstIndex(of: "--host-app"),
            CommandLine.arguments.indices.contains(index + 1)
        {
            hostApp = URL(fileURLWithPath: CommandLine.arguments[index + 1])
        } else {
            hostApp = nil
        }
    }

    func run() {
        emit(["type": "boot"])
        if CommandLine.arguments.contains("--test-wake") {
            emit(["type": "wake", "phrase": "automated cold-launch test"])
            openHostApp()
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.75) {
                exit(0)
            }
            RunLoop.main.run()
            return
        }

        guard let recognizer, recognizer.isAvailable else {
            emit(["type": "error", "message": "speech recognizer unavailable"])
            exit(2)
        }

        let currentAuthorization = SFSpeechRecognizer.authorizationStatus()
        emit([
            "type": "authorization",
            "status": authorizationName(currentAuthorization),
        ])
        if CommandLine.arguments.contains("--status-only") {
            exit(0)
        }
        if currentAuthorization == .authorized {
            startRecognition()
            RunLoop.main.run()
            return
        }

        SFSpeechRecognizer.requestAuthorization { [weak self] status in
            DispatchQueue.main.async {
                guard let self else { return }
                guard status == .authorized else {
                    self.emit([
                        "type": "authorization",
                        "status": self.authorizationName(status),
                    ])
                    exit(3)
                }
                self.emit(["type": "authorization", "status": "authorized"])
                self.startRecognition()
            }
        }

        RunLoop.main.run()
    }

    private func startRecognition() {
        let input = audioEngine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            emit(["type": "error", "message": "microphone input unavailable"])
            exit(4)
        }

        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        request.taskHint = .search
        request.contextualStrings = phrases
        // Follow Apple's normal Speech Recognition path, like Siri's speech
        // input. Requiring the local language pack can leave the listener in
        // a permanent ready state without producing any transcript on macOS.
        request.requiresOnDeviceRecognition = false
        if #available(macOS 13.0, *) {
            request.addsPunctuation = false
        }
        self.request = request

        input.removeTap(onBus: 0)
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in
            request.append(buffer)
        }

        do {
            audioEngine.prepare()
            try audioEngine.start()
        } catch {
            emit(["type": "error", "message": "microphone start failed"])
            exit(5)
        }

        emit(["type": "ready"])
        task = recognizer?.recognitionTask(with: request) { [weak self] result, error in
            guard let self, !self.hasWoken else { return }
            if let result {
                let spoken = self.normalize(result.bestTranscription.formattedString)
                if self.phrases.contains(where: spoken.contains) {
                    self.hasWoken = true
                    self.emit([
                        "type": "wake",
                        "phrase": result.bestTranscription.formattedString,
                    ])
                    self.stop()
                    self.openHostApp()
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.75) {
                        exit(0)
                    }
                    return
                }
            }
            if error != nil {
                self.emit(["type": "error", "message": "speech recognition interrupted"])
                self.stop()
                exit(6)
            }
        }
    }

    private func stop() {
        if audioEngine.isRunning {
            audioEngine.stop()
        }
        audioEngine.inputNode.removeTap(onBus: 0)
        request?.endAudio()
        task?.cancel()
    }

    private func openHostApp() {
        guard let hostApp else { return }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        configuration.arguments = ["--jarvis-wake"]
        NSWorkspace.shared.openApplication(
            at: hostApp,
            configuration: configuration
        ) { [weak self] _, error in
            if let error {
                self?.emit([
                    "type": "error",
                    "message": "could not open Jarvis: \(error.localizedDescription)",
                ])
            }
        }
    }

    private func normalize(_ value: String) -> String {
        value
            .lowercased()
            .replacingOccurrences(of: " ", with: "")
            .replacingOccurrences(of: ",", with: "")
            .replacingOccurrences(of: "，", with: "")
            .replacingOccurrences(of: ".", with: "")
            .replacingOccurrences(of: "。", with: "")
            .replacingOccurrences(of: "！", with: "")
            .replacingOccurrences(of: "!", with: "")
    }

    private func authorizationName(_ status: SFSpeechRecognizerAuthorizationStatus) -> String {
        switch status {
        case .authorized: return "authorized"
        case .denied: return "denied"
        case .restricted: return "restricted"
        case .notDetermined: return "notDetermined"
        @unknown default: return "unknown"
        }
    }

    private func emit(_ value: [String: String]) {
        guard
            let data = try? JSONSerialization.data(withJSONObject: value),
            let line = String(data: data, encoding: .utf8)
        else { return }
        if let eventFile {
            let data = Data((line + "\n").utf8)
            if let handle = try? FileHandle(forWritingTo: eventFile) {
                _ = try? handle.seekToEnd()
                try? handle.write(contentsOf: data)
                try? handle.close()
            } else {
                try? data.write(to: eventFile, options: .atomic)
            }
        } else {
            print(line)
            fflush(stdout)
        }
    }
}

private struct WakeConfig: Decodable {
    let phrases: [String]
}

let listener = WakeListener()
listener.run()
