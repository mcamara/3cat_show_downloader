# 3Cat Plex Metadata

This context maps 3Cat media identities to the identities Plex uses to scan and display TV episodes.

## Language

**3Cat chapter**:
The source chapter identified by 3Cat's `capitol` value and media ID. It remains the stable download and fallback identity.
_Avoid_: Plex episode, TVDB episode

**Plex episode**:
The season and episode pair written into the filename and NFO file that Plex scans.
_Avoid_: 3Cat chapter

**TVDB match**:
A unique normalized-title match, or a unique fuzzy title match scoring at least 0.95, that assigns TheTVDB Aired Order to a 3Cat chapter.
_Avoid_: Guess, approximate rename

**3Cat fallback**:
A Plex episode using season 1 and the original 3Cat chapter number because no safe TVDB match exists.
_Avoid_: Special, unmatched file
