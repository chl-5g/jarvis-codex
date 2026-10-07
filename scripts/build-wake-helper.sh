#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
project_dir=${script_dir:h}
helper_dir="$project_dir/src-tauri/wake-helper"
app_dir="$helper_dir/JarvisWakeListener.app"
binary_dir="$app_dir/Contents/MacOS"
signing_identity=${APPLE_SIGNING_IDENTITY:--}

build_universal_swift_binary() {
  local output=$1
  shift
  local arm_output="${output}.arm64"
  local intel_output="${output}.x86_64"
  /usr/bin/arch -arm64 /usr/bin/swiftc -target arm64-apple-macos13.0 "$@" -o "$arm_output"
  /usr/bin/arch -x86_64 /usr/bin/swiftc -target x86_64-apple-macos13.0 "$@" -o "$intel_output"
  /usr/bin/lipo -create "$arm_output" "$intel_output" -output "$output"
  /bin/rm -f "$arm_output" "$intel_output"
}

mkdir -p "$binary_dir"
mkdir -p "$app_dir/Contents/Resources"
cp "$helper_dir/Info.plist" "$app_dir/Contents/Info.plist"
cp "$project_dir/config/wake.json" "$app_dir/Contents/Resources/wake.json"
build_universal_swift_binary "$binary_dir/JarvisWakeListener" \
  -O \
  -framework AppKit \
  -framework AVFoundation \
  -framework Speech \
  "$helper_dir/JarvisWakeListener.swift"
/usr/bin/codesign \
  --force \
  --options runtime \
  --entitlements "$helper_dir/Entitlements.plist" \
  --sign "$signing_identity" \
  "$app_dir"

location_dir="$project_dir/src-tauri/location-helper"
location_app="$location_dir/JarvisLocationHelper.app"
mkdir -p "$location_app/Contents/MacOS"
cp "$location_dir/Info.plist" "$location_app/Contents/Info.plist"
build_universal_swift_binary "$location_app/Contents/MacOS/JarvisLocationHelper" \
  -O -framework AppKit -framework CoreLocation \
  "$location_dir/JarvisLocationHelper.swift"
/usr/bin/codesign --force --options runtime \
  --entitlements "$location_dir/Entitlements.plist" \
  --sign "$signing_identity" "$location_app"

camera_dir="$project_dir/src-tauri/camera-helper"
camera_app="$camera_dir/JarvisCameraHelper.app"
mkdir -p "$camera_app/Contents/MacOS"
cp "$camera_dir/Info.plist" "$camera_app/Contents/Info.plist"
build_universal_swift_binary "$camera_app/Contents/MacOS/JarvisCameraHelper" \
  -O -framework AVFoundation \
  "$camera_dir/JarvisCameraHelper.swift"
/usr/bin/codesign --force --options runtime \
  --entitlements "$camera_dir/Entitlements.plist" \
  --sign "$signing_identity" "$camera_app"
