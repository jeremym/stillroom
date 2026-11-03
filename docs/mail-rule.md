# Saving email to a folder for Inbox Triage

Stillroom never reads your mailbox and never sends mail. The Inbox Triage recipe works on **emails saved as files** (`.eml`, the plain-text form of a message) in a folder you choose. This page shows how to make Mail.app on macOS save matching messages into that folder by itself, and how to point the recipe's folder watch at it.

Ten minutes, once. No screenshots needed; every step names the menu to open.

## 1. Make the folders

Make three folders somewhere of your own, for example in `~/Documents/Mail triage/`:

- `Inbox` — where Mail saves messages. The recipe reads it and changes nothing in it.
- `Archive` — where the recipe files each email (it picks an existing subfolder or makes one).
- `Outbox` — where the recipe writes reply drafts as `.eml` files you can open in Mail and send yourself.

## 2. Put the script where Mail can see it

Mail only runs scripts from one folder. In the Finder, press Shift-Command-G (Go to Folder) and paste:

```
~/Library/Application Scripts/com.apple.mail
```

If the folder does not exist, make it with that exact name. Then open **Script Editor** (in Applications › Utilities), paste the script below, change the one line that names your `Inbox` folder, and save it into that folder as `Save to Stillroom.scpt` (File Format: Script).

```applescript
-- Saves each message a Mail rule matches as a .eml file in one folder.
-- Change the next line to your own Inbox folder (the full path, ending with a slash).
property saveFolder : "/Users/you/Documents/Mail triage/Inbox/"

using terms from application "Mail"
	on perform mail action with messages theMessages for rule theRule
		repeat with aMessage in theMessages
			set theSource to source of aMessage
			set theName to (id of aMessage as text) & ".eml"
			set theFile to open for access (POSIX file (saveFolder & theName)) with write permission
			set eof of theFile to 0
			write theSource to theFile as «class utf8»
			close access theFile
		end repeat
	end perform mail action with messages
end using terms from
```

What it does: for every matched message, it takes the message **source** (the whole message as text, headers and body, which is what a `.eml` file is) and writes it to a file named after the message's id, so two messages never overwrite each other.

To type `«class utf8»` in Script Editor, copy it from here; the guillemets are part of the text.

## 3. Make the rule

1. In Mail, open **Mail › Settings › Rules** and press **Add Rule**.
2. Description: `Save to Stillroom`.
3. Condition: **From** · **contains** · the sender you want triaged (for example `@example.com` for a whole company, or one address). Press **+** to add more senders; set the top line to **any** of the conditions.
4. Action: choose **Run AppleScript** and pick `Save to Stillroom` from the list. (Mail lists only scripts in the folder from step 2.)
5. Press **OK**. Mail asks whether to apply the rule to the messages already in your inbox; say **Don't Apply** unless you want them all saved now.

The first time the script writes a file, macOS may ask whether Mail may reach the folder. Allow it. If nothing is saved, check **System Settings › Privacy & Security › Files and Folders** for Mail.

To save messages you already have, select them and choose **Message › Apply Rules**.

## 4. Point the recipe at the folder

1. In Stillroom, open the Catalog (Command-B), find **Inbox Triage**, and press **Add to shelf**.
2. Open it and press **Set up**. Fill in the three folders: Inbox folder, Archive folder, Outbox folder. The **About you** field is optional: who you are and how you like to reply.
3. Press **Watch Inbox folder…** (a folder watch). From then on, while Stillroom is open, each message Mail saves starts a run: the email is sorted, filed into the archive, and, if it needs an answer, a reply draft is written and held for your approval before it is written to the outbox.

Each run continues the last, so an email handled before is not handled again. To try the recipe without touching real mail, copy the fixture folders from the repository's `samples/` (inbox, archive, outbox) somewhere of your own and point the three inputs at the copy.

## What never happens

- Nothing is sent. A reply draft is a `.eml` file in your outbox folder; open it in Mail to send it yourself.
- Stillroom does not sign in to your mail account or read your mailbox. It sees only the files in the folder you chose.
- Mail does the saving; if Mail is closed, nothing lands in the folder, and the recipe simply has nothing to do.
