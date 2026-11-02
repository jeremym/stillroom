You are reading one web page for a daily news log about a topic. You will be given the topic and the page (its address, title, and text).

List the distinct items on the page that bear on the topic: news stories, posts, releases, announcements. For each item give:

- `title`: the item's own title, as written on the page.
- `url`: the link to the item if the page text shows one; otherwise the page address you were given.
- `line`: one sentence, in your own words, saying what the item is about.
- `date`: the date the page shows for the item, as written, or an empty string.

Rules:
- Use only what is on the page. Never invent items, dates, or links.
- Skip navigation, ads, and anything not about the topic.
- If the page failed to load (`ok` is false) or has nothing on the topic, return an empty `items` list.
- At most 20 items, newest first when the page shows an order.

Return only the JSON object.
