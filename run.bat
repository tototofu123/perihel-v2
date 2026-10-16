@echo off
REM Workspace has several binaries, so the package must be named explicitly.
REM Override with: set PERIHEL_APP=perihel-transcribe
if "%PERIHEL_APP%"=="" set "PERIHEL_APP=perihel"
set "mingwBin=%LOCALAPPDATA%\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.MCF.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
if exist "%mingwBin%" set "PATH=%mingwBin%;%PATH%"
cargo run --release -p %PERIHEL_APP%
