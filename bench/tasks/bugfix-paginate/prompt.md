Users report that the last page of results goes missing whenever the number
of items is not an exact multiple of the page size (for example 10 items with
3 per page shows only 3 pages). Fix the bug in `pager.py`.

Expected behaviour:
- `page_count(total, per_page)` is the number of pages needed to show every
  item; 0 items means 0 pages.
- `paginate(items, page, per_page)` returns the items on 1-based `page`; a page
  past the end returns an empty list.
- `page` < 1 or `per_page` < 1 raises `ValueError`.

Keep the public functions' names and signatures. Run the existing tests with
`python3 -m unittest` when you are done.
