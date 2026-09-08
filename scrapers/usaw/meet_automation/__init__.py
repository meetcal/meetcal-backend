"""Automated USAW meet start-list + schedule pipeline.

This package watches a USAW meet page for its start-list and schedule PDFs,
drives the existing scrapers to extract the data, validates it, stages it
without touching any database, posts a Slack review message with a preview
link, and ingests to Postgres only after human approval.

It deliberately reuses the existing scrapers under
``scrapers/usaw/final_start_scraper`` and
``scrapers/usaw/owlcms_schedule_scraper`` rather than reimplementing PDF
parsing, and never modifies them.
"""
