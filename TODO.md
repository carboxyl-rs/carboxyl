# TODO

* Text rendering using terminal native text (IN PROGRESS)(using css text suppression: fallback to other solution if that's not fine)

  * Texts often colide and overwrite each other
  * Sometimes text appears briefly; maybe try calling text suppression earlier?
  * Text extraction/render of it should happen every frame to make use smoother (maybe everything should be synced at the frame rate)
  * '--no-native-text' is not being completely respected: shouldn't suppress text when enabled
  * Some texts are rendered even if they aren't on top
  * In some (relatively rare) cases the rendered text comes with a .thing{css} stuff in it
  * Not all kinds of text are being rendered (specific ones)

* Main keys are working but it'd be better if every single key were sent to servo; like (ctrl, shift) alone and combinations

* A fancy browser ui (as ratatui is already implemented)

* Optional hi-res terminal graphics (Sixel / Kitty)

  * Feature detection + ANSI fallback
