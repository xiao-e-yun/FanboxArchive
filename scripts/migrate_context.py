#!/usr/bin/env python3
"""Migrate `configs/fanbox-archive.json` from the v0.8 format to the v0.9 format.

v0.8:
    creators:     { user_id: { creator_id, support_records: { fee: timestamp } } }
    failed_posts: [ PostListItem { id, ... } ]

v0.9:
    creators:     { user_id: { creator_id, last_updated, unimports: { fee: [post_id] } } }
    failed_posts: [ post_id ]

Usage:
    uv run scripts/migrate_context.py [ARCHIVE_DIR]   (default: ./archive)

The original file is kept as `fanbox-archive.json.bak`.
"""

import json
import shutil
import sys
from pathlib import Path

RELATION_PATH = Path("configs/fanbox-archive.json")


def migrate_creator(creator: dict) -> dict:
    if "last_updated" in creator:  # already migrated
        return creator
    records = creator.get("support_records") or {}
    return {
        "creator_id": creator["creator_id"],
        # Every post published before the newest record was already processed.
        "last_updated": max(records.values(), default=0),
        "unimports": {},
    }


def migrate_failed_post(post) -> str:
    return post if isinstance(post, str) else str(post["id"])


def main() -> int:
    archive = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("archive")
    path = archive / RELATION_PATH
    if not path.exists():
        print(f"No context file at {path}, nothing to migrate.")
        return 0

    old = json.loads(path.read_text(encoding="utf-8"))
    new = {
        "creators": {uid: migrate_creator(c) for uid, c in (old.get("creators") or {}).items()},
        "failed_posts": [migrate_failed_post(p) for p in (old.get("failed_posts") or [])],
    }

    if new == old:
        print(f"{path} is already in the new format.")
        return 0

    backup = path.with_name(path.name + ".bak")
    shutil.copy2(path, backup)
    path.write_text(json.dumps(new, ensure_ascii=False), encoding="utf-8")
    print(f"Migrated {len(new['creators'])} creators, {len(new['failed_posts'])} failed posts.")
    print(f"Backup saved to {backup}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
