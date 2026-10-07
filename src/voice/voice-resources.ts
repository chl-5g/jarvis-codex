export type VoiceResources = {
  peer: RTCPeerConnection | null;
  microphoneStream: MediaStream | null;
  remoteStream: MediaStream | null;
  audioContext: AudioContext | null;
  microphoneAnalyser: AnalyserNode | null;
  remoteAnalyser: AnalyserNode | null;
};

export function cleanupVoiceResources(resources: VoiceResources): void {
  resources.peer?.close();
  resources.peer = null;
  resources.microphoneStream?.getTracks().forEach((track) => track.stop());
  resources.remoteStream?.getTracks().forEach((track) => track.stop());
  resources.microphoneStream = null;
  resources.remoteStream = null;
  resources.microphoneAnalyser = null;
  resources.remoteAnalyser = null;
  void resources.audioContext?.close().catch(() => undefined);
  resources.audioContext = null;
}

export function persistVoiceMute(muted: boolean, storageKey: string): void {
  localStorage.setItem(storageKey, String(muted));
}
