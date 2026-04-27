Fix:
- Ctrl-C/ui overall ain't working when the browser is under load (heavy load makes everything unresponsible, find solutions + multithreading?);

It is slow asf. Might be a problem with that architecture or lack of multi-threading?;
Mostly a math issue but --zoom 100 makes the pages bigger than they should be;
Fix some clippy warnings(mostly gfx module);
Get rid of the direct use of libc;
some hardcoded/overengineered stuff and complex solutions when we could just import a crate;
Review/Refactor tests;
