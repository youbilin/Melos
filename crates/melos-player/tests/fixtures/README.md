# Audio fixtures

These files contain an original, generated 440 Hz sine wave (1 second,
44.1 kHz stereo). They contain no third-party recordings and are provided
under CC0-1.0 for use in automated tests.

Generate the files with FFmpeg:

```powershell
ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=44100:duration=1" -ac 2 -c:a pcm_s16le tone.wav
ffmpeg -i tone.wav -c:a flac tone.flac
ffmpeg -i tone.wav -c:a libmp3lame -b:a 128k tone.mp3
ffmpeg -i tone.wav -c:a aac -b:a 128k tone.m4a
ffmpeg -i tone.wav -c:a libvorbis tone.ogg
```
