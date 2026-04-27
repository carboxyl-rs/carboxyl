Try implementing Sixel or Kitty graphics to improve that resolution (in my opinion they must be optional tho);
Servo is immature asf and we get a stack overflow and address boundary under load (google.com) (someone gotta issue it, even though i question why since i haven't got that issue with Servo before);
That issue with Servo demonstrates that our Panic and Signal Handling is really poor; some of the problems could be solved investigating that further. [set_hook; signal-hook; also restoring terminal with ratatui before a graceful exit];
