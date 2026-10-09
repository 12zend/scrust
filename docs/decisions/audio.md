# Audio

Native playback uses rodio (cpal output, symphonia decoders for WAV including ADPCM, and MP3). Each playing sound is a sink with live volume, playback rate `2^(pitch/120)` and a stereo gain pair for the pan effect; starting a sound that is already playing on the same target restarts it. `play sound until done` waits on a completion flag that is updated once per frame.

Without an audio device (and in headless runs) there is no audio engine, exactly like scratch-vm without `audioEngine`: `play sound until done` does not wait and `loudness` is -1. Microphone loudness is not implemented.
