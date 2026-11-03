/* The example goals on the first-run screen: full sentences the planner is known to draft clean, each
   promising only what the engine does today (FIRST-RUN-PLAN §2). One list, shared by the screen and the
   planner-answer fixtures. "Each morning" waits for schedules; mail is a folder a Mail rule saves into. */
export interface ExampleGoal {
  id: string;
  /** The goal, as shown and as filled into the goal box. */
  sentence: string;
  /** Pasted into Built into the recipe when the example carries standing material. */
  material?: string;
  /** The catalog recipe it most resembles, so "see a finished one" can open its page. */
  like: string;
}

export const EXAMPLE_GOALS: readonly ExampleGoal[] = [
  {
    id: "news-brief",
    sentence: "Read the sites I follow about a topic, keep only what is new, and write me a short brief into my notes folder.",
    like: "daily-news-log",
  },
  {
    id: "mail-sorter",
    sentence: "When a saved email lands in a folder, sort it, file it, and draft a reply for me to send.",
    like: "inbox-triage",
  },
  {
    id: "ask-pdfs",
    sentence: "Ask me for a folder of PDFs and a question, and answer from them with the page it came from.",
    like: "ask-your-documents",
  },
  {
    id: "study-kit",
    sentence: "Turn a document into short notes, study cards, and a quiz.",
    like: "learning-kit",
  },
  {
    id: "cited-brief",
    sentence: "Read the links I give it about a question and write a cited brief.",
    like: "research-brief",
  },
];

/** What is not offered, said plainly under a fold on the first screen. One short paragraph. */
export const CANNOT_YET =
  "Send mail or messages. Post to Notion, Slack, or any service: nothing leaves this Mac; a recipe writes files, and your notes app can watch a folder. " +
  "Read a mailbox directly: save messages to a folder with a Mail rule instead. Transcribe a recording. Run on a schedule.";
