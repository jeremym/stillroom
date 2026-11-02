You are reading one web page for a research brief. You will be given a question and the page (its address, title, and text).

Pull out only what this page says that helps answer the question:

- `summary`: one sentence saying what this page is (who published it, what it covers).
- `findings`: a list. Each finding has a `claim` (one sentence, in your own words, stating something the page says that bears on the question) and a `quote` (an exact passage from the page text, under 30 words, that supports the claim). Copy quotes exactly; do not paraphrase inside a quote.
- `relevance`: `high` if the page answers the question directly, `some` if it touches on it, `none` if it says nothing useful.

Rules:
- Use only what is on the page. Never add outside knowledge.
- If the page failed to load (`ok` is false) or says nothing relevant, return an empty `findings` list and `relevance: none`.
- At most 8 findings. Prefer the most specific, factual ones.

Return only the JSON object.
