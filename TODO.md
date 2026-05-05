# TODO

* Text rendering using terminal native text (IN PROGRESS)

  * Texts often colide and overwrite each other
  * (as a polishment) Text extraction/render of it should happen every frame to make use smoother (maybe everything should be synced at the frame rate)
  * Some texts are rendered even if they aren't on top
  * Not all kinds of text are being rendered (specific ones)
  * Performance optimization (on native text, maybe whole browser also)

* Main keys are working but it'd be better if every single key were sent to servo; like (ctrl, shift) alone and combinations

* A fancy browser ui (as ratatui is already implemented)

* Optional hi-res terminal graphics (Sixel / Kitty)

  * Feature detection + ANSI fallback
