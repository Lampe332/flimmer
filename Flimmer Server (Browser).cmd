@echo off
title Flimmer Server (Fenster schliessen = Server aus)
cd /d "%~dp0"
"%~dp0target\release\flimmer-server.exe" --root "%~dp0." --open
