$exe = Join-Path $PSScriptRoot 'campus.exe'
$ico = Join-Path $PSScriptRoot 'campus.ico'
foreach ($f in @($exe, $ico)) {
    if (Test-Path $f) {
        try { Unblock-File -LiteralPath $f -ErrorAction SilentlyContinue } catch {}
    }
}
& $exe @args
exit $LASTEXITCODE
