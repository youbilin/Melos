Melos 0.1.8 — development preview

Extract this folder and run melos.exe on Windows 10 (1903+)/11 (x64).
For terminal playback, first save a connection in the desktop app, then
run melos-tui.exe in PowerShell or Windows Terminal. Use --help for keys.
The terminal preview supports album/playlist browsing, song search,
queue editing, playback modes and lyrics; playlist editing uses the desktop.
Microsoft Visual C++ 2015-2022 Redistributable (x64) must be installed:
https://aka.ms/vs/17/release/vc_redist.x64.exe
Enter your Navidrome / OpenSubsonic server URL, username and password.
Settings are saved in %APPDATA%\melos\settings.json. The password is
protected with Windows DPAPI and is tied to your Windows user account.

Supported local decoders: WAV, FLAC, MP3, AAC/ALAC and Vorbis.
Other formats request server-side MP3 transcoding. Range seeking and
automatic resume require a server that supports byte ranges.

The sidebar can collapse to icons. Playback modes use shuffle and repeat icons.
Shuffle draws random songs from the whole library and keeps going after the
queue ends. Queue entries support next-play insertion, append, removal and
reordering. Song lists and queues render visible rows; use Locate current in
the queue to return to the active track in a long list.
Playlists support browse/play/create/append/remove/rename/confirmed delete.
Click the playback bar artwork to open lyrics; click the lyrics artwork to
return. Timed lines glide to the center, line seeking and plain text work too.
Windows system media controls receive the current song's cached cover image.
Space toggles playback; arrows seek 5 seconds; Ctrl+arrows switch tracks
or adjust volume. Ctrl+S/R/Q/F control shuffle/repeat/queue/search.
If the audio device disconnects, use Settings > Reconnect audio output
in the desktop app, or press e in the terminal. Reconnecting preserves
the current track, upcoming queue and position, and starts paused.

This is a development preview. See the project acceptance record for
remaining manual checks and limitations:
https://github.com/youbilin/Melos/blob/main/docs/03-acceptance.md

Licensed under GPL-3.0-or-later. See LICENSE and THIRD_PARTY_NOTICES.md.
