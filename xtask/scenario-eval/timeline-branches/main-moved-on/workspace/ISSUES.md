# Open issues

## 1. Sizes without a unit are read as kilobytes

`parse.parse_size("12")` returns 12288; it should return 12. The suffixes
`kb`, `mb` and `gb` multiply by 1024, 1024 ** 2 and 1024 ** 3; a plain number
is bytes.

## 2. Durations are not padded

`format.format_duration(185)` returns "3m5s"; it should be "3m05s": the
seconds are always two digits. Hours are not shown, and the minutes are not
padded.

## 3. Usernames may start with a digit

`validate.valid_username("1alice")` returns True; it should be False. A
username starts with a letter, may contain letters, digits and underscores,
and is 1 to 20 characters long.
