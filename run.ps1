$mingwBin = "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.MCF.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
if (Test-Path $mingwBin) {
    $env:Path = "$mingwBin;$env:Path"
}
if ($args[0] -eq "run") {
    cargo run --release
} elseif ($args[0] -eq "build") {
    cargo build --release
} elseif ($args[0] -eq "check") {
    cargo check
} else {
    cargo run --release
}
