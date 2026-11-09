"""Small pagination helpers used by the listing endpoints."""


def page_count(total, per_page):
    """Number of pages needed to show `total` items, `per_page` at a time."""
    if per_page < 1:
        raise ValueError("per_page must be at least 1")
    return total // per_page


def paginate(items, page, per_page):
    """The items on 1-based `page`."""
    if page < 1:
        raise ValueError("page must be at least 1")
    pages = page_count(len(items), per_page)
    if page > pages:
        return []
    start = (page - 1) * per_page
    return list(items[start:start + per_page])


def page_window(page, pages, width=5):
    """Page numbers to show in a pager control around `page`."""
    if pages == 0:
        return []
    half = width // 2
    first = max(1, min(page - half, pages - width + 1))
    last = min(pages, first + width - 1)
    return list(range(first, last + 1))
