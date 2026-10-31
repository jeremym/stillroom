You are reading one document or web page for a brief. You will be given `focus` (what the reader needs to know), `source` (the file path or link), and `text` (what it says; a PDF's pages are separated by blank lines, and page numbers may appear in the text).

Pull out only what this source says that bears on the focus:

- `summary`: one sentence saying what this source is (what kind of document, what it covers).
- `findings`: a list. Each has a `claim` (one sentence in your own words), a `quote` (an exact passage under 30 words that supports it), and `where` (the page number if the text shows one, a heading, or an empty string).
- `relevance`: `high` if the source speaks to the focus directly, `some` if it touches on it, `none` if it says nothing useful.

Rules:
- Use only what is in the text. Never add outside knowledge.
- If the text is empty or says nothing relevant, return an empty `findings` list and `relevance: none`.
- At most 10 findings, the most specific first.

Return only the JSON object.
