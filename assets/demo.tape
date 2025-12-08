Output "demo.gif"

Require echo

Set Shell "zsh"
Set FontSize 18
Set Width 1920
Set Height 1080
Set TypingSpeed 50ms
Set FontFamily "Hack Nerd Font"
Set Padding 16
Set WindowBar Colorful
Set Framerate 20

Hide
Type "clear"
Enter
Show

Type `mq-tui demo.md`
Enter
Sleep 1.5s

Type `s`
Sleep 1s

Type `j`
Sleep 0.5s

Type `j`
Sleep 0.5s

Type `j`
Sleep 0.5s

Type `j`
Sleep 0.5s

Type `j`
Sleep 1s

Type `s`
Sleep 1s

Type `:`
Sleep 0.5s

Type `.h`
Sleep 1.5s

Backspace 2
Sleep 1.5s

Type `.code`
Sleep 1.5s

Backspace 5
Sleep 1.5s

Type `select(.code.lang == "python")`
Sleep 1.5s

Escape
Sleep 1.5s
