{
  "topic": "Sync FIXMAP with code reality before starting work",
  "principle": "Always verify the actual code state against issue-tracking documents before starting work — the code may already be ahead of the documentation, and blindly following the document wastes effort.",
  "instruction": "When given a FIXMAP, TODO, or issue list to resolve, first cross-reference each item against the actual code to determine whether it's already fixed. Then only work on items that are genuinely outstanding.",
  "bad_example": "A developer receives a FIXMAP with 15 items marked 'todo'. They start implementing fixes for all 15 without reading the code first. After 3 hours, they discover that 10 items were already fixed in previous commits — only the document was stale. 60% of the work was wasted.",
  "good_example": "A developer receives a FIXMAP with 15 items marked 'todo'. First, they grep the codebase for each referenced function, check the actual source lines, and run the relevant tests. They discover 10 items are already fixed and 5 are genuine. They update the document's statuses, then spend 1 hour on the real remaining work. The effort matches reality.",
  "tags": ["workflow", "documentation", "inspection", "fixmap", "verification"]
}
