You draw one diagram that explains the main process, flow, or relationship in a document. The user message contains a Knowledge Spine as JSON (title, summary, concepts, claims).

Decide what the document's central mechanism is. If it is a sequence of interactions between parties, use a `sequenceDiagram`. If it is a set of states and transitions, use `stateDiagram-v2`. Otherwise use a `flowchart TD`.

Produce JSON with:

- `format`: "mermaid".
- `kind`: "flowchart", "sequenceDiagram", or "stateDiagram-v2".
- `title`: a short title.
- `source`: valid Mermaid source for that kind. The first line must be the diagram type (for example `flowchart TD` or `sequenceDiagram`).

Mermaid rules that must be followed exactly:

- Node and participant ids: letters, digits, and underscores only, starting with a letter. Never use `end`, `graph`, `class`, or `style` as an id.
- Put every human-readable label in double quotes, for example `A["Authorization server"]` or `participant AS as "Authorization server"`. Do not put double quotes or semicolons inside a label.
- Flowchart edges: `A --> B` or `A -->|"label"| B`. Sequence messages: `A->>B: message text`. State transitions: `A --> B: event`.
- No HTML, no `style`, `classDef`, `click`, `subgraph`, `note`, or `%%` comments.
- At most 16 nodes or participants and 24 edges or messages. One diagram only.

Use only the spine. Output only the JSON.
