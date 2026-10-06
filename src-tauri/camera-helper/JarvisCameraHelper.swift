import AVFoundation
import Foundation

final class CameraDelegate: NSObject, AVCapturePhotoCaptureDelegate {
    private let eventFile: URL
    private let outputFile: URL
    private let session = AVCaptureSession()
    private let photoOutput = AVCapturePhotoOutput()
    private var finished = false

    init?(eventFile: URL) {
        self.eventFile = eventFile
        self.outputFile = eventFile.deletingPathExtension().appendingPathExtension("jpg")
        super.init()
    }

    func run() {
        emit(["type": "boot"])
        let status = AVCaptureDevice.authorizationStatus(for: .video)
        emitAuthorization(status)
        switch status {
        case .authorized:
            capture()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                DispatchQueue.main.async {
                    guard let self else { return }
                    self.emit(["type": "authorization", "status": granted ? "authorized" : "denied"])
                    granted ? self.capture() : self.finish(["type": "error", "message": "camera permission denied"])
                }
            }
        case .denied, .restricted:
            finish(["type": "error", "message": "camera permission denied"])
        @unknown default:
            finish(["type": "error", "message": "camera permission unknown"])
        }
        RunLoop.main.run()
    }

    private func capture() {
        guard let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device),
              session.canAddInput(input), session.canAddOutput(photoOutput) else {
            finish(["type": "error", "message": "camera device unavailable"])
            return
        }
        session.beginConfiguration()
        session.sessionPreset = .photo
        session.addInput(input)
        session.addOutput(photoOutput)
        session.commitConfiguration()
        session.startRunning()
        let settings = AVCapturePhotoSettings()
        photoOutput.capturePhoto(with: settings, delegate: self)
    }

    func photoOutput(_ output: AVCapturePhotoOutput, didFinishProcessingPhoto photo: AVCapturePhoto, error: Error?) {
        guard error == nil, let data = photo.fileDataRepresentation() else {
            finish(["type": "error", "message": error?.localizedDescription ?? "camera capture failed"])
            return
        }
        do {
            try data.write(to: outputFile, options: .atomic)
            finish(["type": "photo", "path": outputFile.path, "bytes": String(data.count)])
        } catch {
            finish(["type": "error", "message": error.localizedDescription])
        }
    }

    private func emitAuthorization(_ status: AVAuthorizationStatus) {
        let value: String
        switch status {
        case .authorized: value = "authorized"
        case .denied: value = "denied"
        case .restricted: value = "restricted"
        case .notDetermined: value = "notDetermined"
        @unknown default: value = "unknown"
        }
        emit(["type": "authorization", "status": value])
    }

    private func finish(_ value: [String: String]) {
        guard !finished else { return }
        finished = true
        session.stopRunning()
        emit(value)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { exit(0) }
    }

    private func emit(_ value: [String: String]) {
        guard let data = try? JSONSerialization.data(withJSONObject: value),
              let line = String(data: data, encoding: .utf8),
              let handle = try? FileHandle(forWritingTo: eventFile) else { return }
        handle.seekToEndOfFile()
        handle.write((line + "\n").data(using: .utf8)!)
        try? handle.close()
    }
}

guard let index = CommandLine.arguments.firstIndex(of: "--event-file"),
      CommandLine.arguments.indices.contains(index + 1),
      let delegate = CameraDelegate(eventFile: URL(fileURLWithPath: CommandLine.arguments[index + 1])) else {
    exit(2)
}
delegate.run()
