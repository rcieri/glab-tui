Output assets/demo-selection.gif

Set FontSize 18
Set FontFamily "JetBrainsMono Nerd Font"
Set Theme "Builtin Dark"
Set Width 1920
Set Height 1200
Set Framerate 10

Type "glab-tui"
Enter
Sleep 3s

# Select an item in the list
Type "j"
Sleep 400ms
Type "j"
Sleep 400ms
Type "j"
Sleep 600ms

# Open the edit menu
Type "e"
Sleep 2s

# Navigate through edit fields (title, labels, assignees, milestone, ...)
Type "j"
Sleep 500ms
Type "j"
Sleep 500ms
Type "j"
Sleep 500ms
Type "k"
Sleep 500ms

# Open the Labels selector overlay
Enter
Sleep 1s

# Toggle a label with Space
Type " "
Sleep 500ms

# Confirm label selection
Enter
Sleep 1s

# Close the edit menu
Escape
Sleep 500ms

# Quit
Type "q"
Sleep 500ms
