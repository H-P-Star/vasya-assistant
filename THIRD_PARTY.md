# Bundled speech components

Vosk API 0.3.45: Copyright Alpha Cephei and contributors, Apache License 2.0.
Source and build instructions: https://github.com/alphacep/vosk-api/tree/v0.3.45

Russian model vosk-model-small-ru-0.22: Alpha Cephei, Apache License 2.0.
Model source and license listing: https://alphacephei.com/vosk/models
The original model README is retained in the installed model directory.

libstdc++-6.dll and libgcc_s_seh-1.dll are GCC runtime libraries, distributed
under GPLv3 with the GCC Runtime Library Exception 3.1. GCC sources:
https://gcc.gnu.org/git.html ; https://gcc.gnu.org/onlinedocs/libstdc++/manual/license.html

libwinpthread-1.dll is from mingw-w64 winpthreads. Its license notices are
reproduced in licenses/mingw-w64-COPYING; sources:
https://github.com/mingw-w64/mingw-w64/tree/master/mingw-w64-libraries/winpthreads

License texts are included in the licenses directory of the installation.
Archive URLs and verified SHA256 digests are pinned in scripts/speech-assets.json.

Whisper.cpp 1.8.2 and ggml: Copyright Georgi Gerganov and contributors, MIT.
Sources and pinned release: https://github.com/ggml-org/whisper.cpp/tree/v1.8.2
Multilingual Whisper base weights: Copyright OpenAI, MIT.
Model distribution: https://huggingface.co/ggerganov/whisper.cpp ; original project: https://github.com/openai/whisper
Notices: licenses/whisper-LICENSE and licenses/whisper-model-LICENSE.
The optional SDL2 runtime in the upstream archive is not bundled.

Piper is an optional external executable chosen by the user; no Piper binary or
voice model is distributed in this installer. Each downloaded voice has its own
model card and license; users can use Windows voices without Piper.

## Markdown rendering

Markdown-it and its dependencies are distributed under the notices reproduced
in licenses/markdown-frontend-NOTICES.txt. Source: https://github.com/markdown-it/markdown-it
