# The root window hides instead of taking over another document

The first window an app opens is the framework's root window, and the app ends when it closes. WordCraft
used to keep it open when the user closed it with other document windows open: it switched to another
window's document and that window disappeared, so the window the user closed stayed and a different one
vanished (#7752). We decided instead that the root window hides (storytold/craft-libs#584) and adopts
nothing, so the window the user closes is always the one that goes, and the app ends only when the last
document window closes or the user quits.

## Considered options

- **Root takes over another document** (the old behaviour): no framework change, but it is the reported bug,
  and the moved window loses its position and size.
- **Root shows no document and stays visible**: leaves an empty window the user did not ask for.
