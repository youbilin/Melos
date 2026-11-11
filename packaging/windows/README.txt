Melos 0.1.1 — development preview

Extract this folder and run melos.exe on Windows 10 (1903+)/11 (x64).
Microsoft Visual C++ 2015-2022 Redistributable (x64) must be installed:
https://aka.ms/vs/17/release/vc_redist.x64.exe
Enter your Navidrome / OpenSubsonic server URL, username and password.
Settings are saved in %APPDATA%\melos\settings.json. The password is
protected with Windows DPAPI and is tied to your Windows user account.

Supported local decoders: WAV, FLAC, MP3, AAC/ALAC and Vorbis.
Other formats request server-side MP3 transcoding. Range seeking and
automatic resume require a server that supports byte ranges.

Playback bar: shuffle, repeat off/all/one, Lyrics and Queue.
Queue entries support next-play insertion, append, removal and reordering.
Playlists support browse/play/create/append/remove/rename/confirmed delete.
Lyrics support timed highlighting, following, line seeking and plain text.
Space toggles playback; arrows seek 5 seconds; Ctrl+arrows switch tracks
or adjust volume. Ctrl+S/R/Q/F control shuffle/repeat/queue/search.

This is a development preview. See the project acceptance record for
remaining manual checks and limitations:
https://github.com/youbilin/Melos/blob/main/docs/03-acceptance.md

Licensed under GPL-3.0-or-later. See LICENSE and THIRD_PARTY_NOTICES.md.
