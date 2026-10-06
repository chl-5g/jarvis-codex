import AppKit
import CoreLocation
import Foundation

final class LocationDelegate: NSObject, CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    private let eventFile: URL
    private var finished = false

    init?(eventFile: URL) {
        self.eventFile = eventFile
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyKilometer
    }

    func run() {
        emit(["type": "boot"])
        emitAuthorization(manager.authorizationStatus)
        switch manager.authorizationStatus {
        case .authorized, .authorizedAlways, .authorizedWhenInUse:
            manager.requestLocation()
        case .notDetermined:
            manager.requestWhenInUseAuthorization()
        case .denied, .restricted:
            finish(["type": "error", "message": "location permission denied"])
        @unknown default:
            finish(["type": "error", "message": "location permission unknown"])
        }
        RunLoop.main.run()
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        emitAuthorization(manager.authorizationStatus)
        switch manager.authorizationStatus {
        case .authorized, .authorizedAlways, .authorizedWhenInUse:
            manager.requestLocation()
        case .denied, .restricted:
            finish(["type": "error", "message": "location permission denied"])
        default:
            break
        }
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let location = locations.first else {
            finish(["type": "error", "message": "location unavailable"])
            return
        }
        CLGeocoder().reverseGeocodeLocation(location) { [weak self] placemarks, _ in
            guard let self else { return }
            let place = placemarks?.first
            self.finish([
                "type": "location",
                "latitude": String(location.coordinate.latitude),
                "longitude": String(location.coordinate.longitude),
                "city": place?.locality ?? "",
                "region": place?.administrativeArea ?? "",
                "country": place?.country ?? "",
                "source": "core_location",
            ])
        }
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        finish(["type": "error", "message": error.localizedDescription])
    }

    private func emitAuthorization(_ status: CLAuthorizationStatus) {
        let value: String
        switch status {
        case .authorized, .authorizedAlways, .authorizedWhenInUse: value = "authorized"
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
        emit(value)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { exit(0) }
    }

    private func emit(_ value: [String: String]) {
        guard let data = try? JSONSerialization.data(withJSONObject: value),
              let line = String(data: data, encoding: .utf8) else { return }
        guard let handle = try? FileHandle(forWritingTo: eventFile) else { return }
        handle.seekToEndOfFile()
        handle.write((line + "\n").data(using: .utf8)!)
        try? handle.close()
    }
}

guard let index = CommandLine.arguments.firstIndex(of: "--event-file"),
      CommandLine.arguments.indices.contains(index + 1),
      let delegate = LocationDelegate(eventFile: URL(fileURLWithPath: CommandLine.arguments[index + 1])) else {
    exit(2)
}
delegate.run()
