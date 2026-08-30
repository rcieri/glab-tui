Output assets/demo-diff.gif

Set FontSize 18
Set FontFamily "JetBrainsMono Nerd Font"
Set Theme "catppuccin-mocha"
Set Width 1920
Set Height 1200
Set Framerate 10

Type "glab-tui"
Enter
Sleep 3s

# Navigate to Merge Requests tab
Type "l"
Sleep 1s

# Select a merge request
Type "j"
Sleep 400ms
Type "j"
Sleep 600ms

# Open Diff View with 'D'
Type "D"
Sleep 2s

# Navigate the file tree
Type "j"
Sleep 500ms
Type "j"
Sleep 500ms

# Focus into diff pane
Enter
Sleep 1s

# Scroll through diff lines
Type "j"
Sleep 300ms
Type "j"
Sleep 300ms
Type "j"
Sleep 300ms
Type "j"
Sleep 500ms

# Toggle side-by-side diff view
Type "d"
Sleep 2s

# Scroll side-by-side view
Type "j"
Sleep 300ms
Type "j"
Sleep 300ms
Type "k"
Sleep 500ms

# Mark file as reviewed
Type "m"
Sleep 1s

# Switch back to unified diff view
Type "d"
Sleep 1s

# Close Diff View
Escape
Sleep 800ms

# Quit
Type "q"
Sleep 500ms
