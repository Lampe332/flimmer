@echo off
title Flimmer (Fenster schliessen = Server aus)
cd /d "%~dp0"
"%~dp0server\target\release\flimmer.exe" --root "%~dp0." --open
