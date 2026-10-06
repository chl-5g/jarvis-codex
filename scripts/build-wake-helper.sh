#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
project_dir=${script_dir:h}
helper_dir="$project_dir/src-tauri/wake-helper"
app_dir="$helper_dir/JarvisWakeListener.app"
binary_dir="$app_dir/Contents/MacOS"
signing_identity=${APPLE_SIGNING_IDENTITY:--}

mkdir -p "$binary_dir"
cp "$helper_dir/Info.plist" "$app_dir/Contents/Info.plist"
/usr/bin/swiftc \
  -O \
  -framework AppKit \
  -framework AVFoundation \
  -framework Speech \
  "$helper_dir/JarvisWakeListener.swift" \
  -o "$binary_dir/JarvisWakeListener"
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
/usr/bin/swiftc -O -framework AppKit -framework CoreLocation \
  "$location_dir/JarvisLocationHelper.swift" \
  -o "$location_app/Contents/MacOS/JarvisLocationHelper"
/usr/bin/codesign --force --options runtime \
  --entitlements "$location_dir/Entitlements.plist" \
  --sign "$signing_identity" "$location_app"

camera_dir="$project_dir/src-tauri/camera-helper"
camera_app="$camera_dir/JarvisCameraHelper.app"
mkdir -p "$camera_app/Contents/MacOS"
cp "$camera_dir/Info.plist" "$camera_app/Contents/Info.plist"
/usr/bin/swiftc -O -framework AVFoundation \
  "$camera_dir/JarvisCameraHelper.swift" \
  -o "$camera_app/Contents/MacOS/JarvisCameraHelper"
/usr/bin/codesign --force --options runtime \
  --entitlements "$camera_dir/Entitlements.plist" \
  --sign "$signing_identity" "$camera_app"
