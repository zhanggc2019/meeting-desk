import { FileAudio, X } from "lucide-react";

interface MeetingAudioPlayerProps {
  sourceUrl: string;
  title: string;
  onClose: () => void;
  onPlaybackError: () => void;
}

/** 在应用内流式播放用户原始录音，并提供明确的关闭操作。 */
export function MeetingAudioPlayer({ sourceUrl, title, onClose, onPlaybackError }: MeetingAudioPlayerProps) {
  return (
    <section className="meeting-audio-player" role="region" aria-label={`正在试听 ${title}`}>
      <div className="audio-player-identity">
        <span className="audio-player-icon" aria-hidden="true"><FileAudio size={17} /></span>
        <div><strong>{title}</strong><span>应用内试听</span></div>
      </div>
      <audio key={sourceUrl} controls autoPlay preload="metadata" src={sourceUrl} onError={onPlaybackError} />
      <button className="icon-button" type="button" aria-label="关闭试听" title="关闭试听" onClick={onClose}><X size={16} /></button>
    </section>
  );
}
