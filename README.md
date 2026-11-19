# Fanbox Archive
[Get Started](https://github.com/xiao-e-yun/FanboxArchive/wiki)

> Check [PostArchiver](https://github.com/xiao-e-yun/PostArchiver) to know more info.

It is importer for fanbox to PostArchiver.

```sh
Usage: fanbox-archive [OPTIONS] <SESSION> [OUTPUT]

Arguments:
  <SESSION>  Your `FANBOXSESSID` cookie [env: FANBOXSESSID]
  [OUTPUT]   Which you path want to save [env: OUTPUT=] [default: ./archive]

Options:
  -s, --save <SAVE>                 Which you type want to save [env: SAVE=] [default: supporting] [possible values: all, following, supporting]
      --strategy <STRATEGY>         Archiving strategy [default: increment]
  -w, --whitelist [<WHITELIST>...]  Whitelist of creator IDs
  -b, --blacklist [<BLACKLIST>...]  Blacklist of creator IDs
      --limit <LIMIT>               Limit fetch number of posts per minute [default: 60]
      --skip-free                   Skip free post
      --user-agent <user-agent>     User agent when blocking
      --cookies <cookies>           Custom cookies.  Exapmle: `name=value; name2=value2; ...`  (cf_clearance is required for blocking)
  -v, --verbose...                  Increase logging verbosity
  -q, --quiet...                    Decrease logging verbosity
  -h, --help                        Print help
```

## Build

How to build & run code
```sh
cargo run
```
