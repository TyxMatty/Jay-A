# Writes two speech recordings with Windows' own voices (48 kHz, 16-bit, mono) for the noise
# removal evaluation (crates/jay-audio, `denoise_evaluation`): speech.wav, the person talking
# (Maria, pt-BR), and babble.wav, someone talking in the background (Zira, en-US).
#
#   powershell -File scripts/make-speech.ps1 <folder>
#   $env:JAYA_SPEECH_DIR = '<folder>'; cd crates/jay-audio; cargo test --release -- --ignored denoise_evaluation --nocapture
param([Parameter(Mandatory)] [string] $Folder)

Add-Type -AssemblyName System.Speech
New-Item -ItemType Directory -Force $Folder | Out-Null
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(48000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)

function Say([string] $voice, [string] $text, [string] $file) {
  $s = New-Object System.Speech.Synthesis.SpeechSynthesizer
  $s.SelectVoice($voice)
  $s.SetOutputToWaveFile((Join-Path $Folder $file), $format)
  $s.Speak($text)
  $s.Dispose()
}

Say 'Microsoft Maria Desktop' ('Oi, pessoal, tudo bem? Hoje eu quero mostrar como ficou o filtro de ruído do Jay-A. ' +
  'A ideia é que vocês ouçam só a minha voz, sem o teclado, sem os cliques do mouse e sem ninguém falando atrás de mim. ' +
  'Se der certo, a chamada fica bem mais limpa e ninguém precisa ficar no mudo o tempo todo.') 'speech.wav'
Say 'Microsoft Zira Desktop' ('Did you see the game last night? I think the second half was much better than the first one. ' +
  'Anyway, we should order lunch soon, I am getting hungry. What do you want to eat today? Pizza again?') 'babble.wav'
Get-ChildItem $Folder -Filter *.wav | ForEach-Object { '{0}: {1:N0} bytes' -f $_.Name, $_.Length }
