# wanikani review count simulator

Simulates future [wanikani](https://wanikani.com/) review counts, based on your
past review results.

To run, first you'll need to [setup an API
token](https://www.wanikani.com/settings/personal_access_tokens) and download
your past review results:

    pip install --user -r requirements.txt
    WANIKANI_API_KEY=<wanikani token goes here> python update_cache.py

Then, run wksim (this may take a while):

    cargo run --release
