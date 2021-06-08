import json
import os
import sqlite3
from ast import literal_eval

from dotenv import load_dotenv
from tqdm import tqdm
from wanikani_api.client import Client

OBJECT_TYPES = ("reviews", "subjects", "assignments")


def get_last_object_time(db, obj_type):
    cur = db.cursor()
    cur.execute(f"select max(json_extract(data, '$.data_updated_at')) from {obj_type}")
    row = cur.fetchone()
    if row:
        return row[0]
    else:
        return None


def connect_to_database():
    db = sqlite3.connect("wanikani_cache.db")

    for obj_type in OBJECT_TYPES:
        db.execute(
            f"""
            CREATE TABLE IF NOT EXISTS {obj_type}(
                id integer,
                object text,
                data json,
                primary key(id)
            );
            """
        )

    return db


def setup_wanikani_client():
    api_key = os.environ["WANIKANI_API_KEY"]
    return Client(api_key)


def fix_json(s):
    return json.dumps(literal_eval(s))


def update_cache(db):
    print("Updating cache")

    wk_client = setup_wanikani_client()

    for obj_type in OBJECT_TYPES:
        last_obj_time = get_last_object_time(db, obj_type)
        query_method = getattr(wk_client, obj_type)
        new_objs = query_method(updated_after=last_obj_time, fetch_all=True)

        cur = db.cursor()
        cur.executemany(
            f"insert or replace into {obj_type}(id, object, data) values(?, ?, ?)",
            (
                (obj.id, obj.resource, fix_json(obj.raw_json()))
                for obj in tqdm(new_objs)
            ),
        )
        cur.execute("commit")

    print("Done!")


def main():
    load_dotenv()

    db = connect_to_database()
    update_cache(db)


if __name__ == "__main__":
    main()
