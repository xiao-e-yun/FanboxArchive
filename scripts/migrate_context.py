#!/usr/bin/env python3
"""Migrate `configs/fanbox-archive.json` from the v0.8 format to the v0.9 format.

v0.8:
    creators:     { user_id: { creator_id, support_records: { fee: timestamp } } }
    failed_posts: [ PostListItem { id, feeRequired, creatorId, user: { userId } } ]

v0.9:
    creators:     { user_id: { creator_id, last_updated, unimports: { fee: [post_id] } } }

Failed posts are moved into their creator's `unimports` under fee 0, so they are
retried on the next run regardless of the current plan. If a post turns out to be
restricted, the archiver re-files it under its real fee.

Usage:
    uv run scripts/migrate_context.py [ARCHIVE_DIR]   (default: ./archive)

The original file is kept as `fanbox-archive.json.bak`.
"""

import json
import shutil
import sys
from pathlib import Path

RELATION_PATH = Path("configs/fanbox-archive.json")
RETRY_FEE = "0"


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


def main() -> int:
    archive = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("archive")
    path = archive / RELATION_PATH
    if not path.exists():
        print(f"No context file at {path}, nothing to migrate.")
        return 0

    old = json.loads(path.read_text(encoding="utf-8"))
    if "failed_posts" not in old and all(
        "last_updated" in c for c in (old.get("creators") or {}).values()
    ):
        print(f"{path} is already in the new format.")
        return 0

    creators = {uid: migrate_creator(c) for uid, c in (old.get("creators") or {}).items()}

    moved = skipped = 0
    for post in old.get("failed_posts") or []:
        try:
            user_id = str(post["user"]["userId"])
            creator_id = post["creatorId"]
            post_id = str(post["id"])
        except (KeyError, TypeError):
            skipped += 1
            continue
        creator = creators.setdefault(
            user_id, {"creator_id": creator_id, "last_updated": 0, "unimports": {}}
        )
        retry = creator["unimports"].setdefault(RETRY_FEE, [])
        if post_id not in retry:
            retry.append(post_id)
        moved += 1

    backup = path.with_name(path.name + ".bak")
    shutil.copy2(path, backup)
    path.write_text(json.dumps({"creators": creators}, ensure_ascii=False), encoding="utf-8")

    print(f"Migrated {len(creators)} creators, moved {moved} failed posts into unimports.")
    if skipped:
        print(f"Skipped {skipped} failed posts with missing fields.")
    print(f"Backup saved to {backup}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
