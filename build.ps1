# Puts MinGW (needed by some transitive C/C++ deps) on PATH, then forwards to cargo.
# The workspace has three binaries, so name the package you want, e.g.
#   .\build.ps1 build --release -p perihel
#   .\build.ps1 test
$mingwBin = "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.MCF.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
if (Test-Path $mingwBin) {
    $env:Path = "$mingwBin;$env:Path"
}
cargo @args