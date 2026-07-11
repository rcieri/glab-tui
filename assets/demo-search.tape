Output assets/demo-search.gif

Set FontSize 18
Set FontFamily "JetBrainsMono Nerd Font"
Set Theme "Builtin Dark"
Set Width 1920
Set Height 1200
Set Framerate 10

Type "glab-tui"
Enter
Sleep 3s

# Open fuzzy search bar
Type "/"
Sleep 500ms

# Type a query — shows live fuzzy filtering across all columns
Type "feature"
Sleep 2s
Sleep 500ms

# Close search bar
Escape
Sleep 1s

# Open column configure popup
Tab
Sleep 2s

# Navigate through column list
Type "j"
Sleep 500ms
Type "j"
Sleep 500ms
Type "k"
Sleep 500ms
Type "k"
Sleep 500ms

# Close config popup
Escape
Sleep 500ms

# Quit
Type "q"
Sleep 500ms
