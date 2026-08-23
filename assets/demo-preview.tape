Output assets/demo-preview.gif

Set FontSize 18
Set FontFamily "JetBrainsMono Nerd Font"
Set Theme "nord"
Set Width 1920
Set Height 1200
Set Framerate 10

Type "glab-tui"
Enter
Sleep 3s

# Select an issue in the list
Type "j"
Sleep 400ms
Type "j"
Sleep 400ms
Type "j"
Sleep 600ms

# Enter opens the preview pane
Enter
Sleep 2s

# Enter again zooms the preview
Enter
Sleep 1s

# Scroll through the zoomed preview
Type "J"
Sleep 500ms
Type "J"
Sleep 500ms
Type "K"
Sleep 800ms

# Escape steps back out of zoom, then closes the preview
Escape
Sleep 500ms
Escape
Sleep 800ms

# Quit
Type "q"
Sleep 500ms
