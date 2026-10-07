CREATE STATISTICS IF NOT EXISTS tracks_audio_fingerprint_prefix_stats ON (substr(audio_fingerprint, 1, 64)) FROM tracks;
