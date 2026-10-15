BlockForge {{VERSION}} for Windows (portable)
=============================================

BlockForge is a fast, modern Minecraft launcher.
https://github.com/riyadmondol2006/BlockForge

How to start it
  Unpack this zip anywhere you like (for example a folder on your desktop) and double-click blockforge.exe.
  Nothing is installed, and there are no shortcuts. If you would like those, use the
  BlockForge-{{VERSION}}-windows-x64-setup.exe installer instead.

"Windows protected your PC"
  BlockForge isn't signed with a paid certificate yet, so Windows SmartScreen may warn about it the first time.
  Choose "More info", then "Run anyway".

Where your data is
  Instances, accounts, settings and downloads are kept in %APPDATA%\BlockForge, not next to blockforge.exe, so you can
  delete or replace this folder without losing them. The installed edition and the older Java edition of BlockForge
  use the same folder.
  To keep everything next to the program instead, start it from a Command Prompt with
      set BLOCKFORGE_DIR=D:\BlockForge\data
      blockforge.exe
  (any folder works).

From a terminal
  blockforge.exe --help        lists the commands (list, launch, create, search, add, import, ...)
  blockforge.exe --version

  The window opens without a console, so a terminal doesn't wait for the command to finish. In Command Prompt use
      start /wait blockforge.exe list
  and in PowerShell
      blockforge.exe list | Out-Host

Updating
  Download the newer zip and replace blockforge.exe. Your data stays where it is.

License: GPL-3.0-or-later.
