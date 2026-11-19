# Fanbox Archive
[Get Started](https://github.com/xiao-e-yun/FanboxArchive/wiki)

> Check [PostArchiver](https://github.com/xiao-e-yun/PostArchiver) to know more info.

It is importer for fanbox to PostArchiver.

```sh
Usage: fanbox-archive [OPTIONS] <SESSION>

Arguments:
  <SESSION>
          Your `FANBOXSESSID` cookie
          
          [env: FANBOXSESSID]

Options:
  -o, --output <OUTPUT>
          Archive directory to save into
          
          [env: OUTPUT=]
          [default: ./archive]

  -s, --save <SAVE>
          Which creators to archive
          
          [env: SAVE=]
          [default: supporting]

          Possible values:
          - all:        Creators you follow or support
          - following:  Creators you follow; posts you can't view are skipped
          - supporting: Creators you support with an active plan

      --strategy <STRATEGY>
          Archiving strategy
          
          [default: increment]

          Possible values:
          - increment: Only fetch posts newer than the last run, and skip posts already in the archive
          - full:      Fetch everything again and re-import posts already in the archive

  -w, --whitelist [<WHITELIST>...]
          Only archive these creators (the `name` in `name.fanbox.cc`)

  -b, --blacklist [<BLACKLIST>...]
          Skip these creators (the `name` in `name.fanbox.cc`)

      --limit <LIMIT>
          Limit fetch number of posts per minute
          
          [default: 60]

      --skip-free
          Skip free post

      --user-agent <user-agent>
          User agent when blocking

      --cookies <cookies>
          Custom cookies.  Exapmle: `name=value; name2=value2; ...`  (cf_clearance is required for blocking)

  -v, --verbose...
          Increase logging verbosity

  -q, --quiet...
          Decrease logging verbosity

  -h, --help
          Print help (see a summary with '-h')
```

## Build

How to build & run code
```sh
cargo run
```
