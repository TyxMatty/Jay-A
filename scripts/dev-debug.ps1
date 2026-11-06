# Starts `npm run dev` with WebView2 remote debugging on port 9224, so scripts/cdp-eval.mjs can
# drive the real app (Jay-C's script; Jay-C uses 9222, so both can run at once).
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9224'
Set-Location (Split-Path $PSScriptRoot -Parent)
npm run dev
