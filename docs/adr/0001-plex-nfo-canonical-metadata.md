# Use NFO as canonical Plex metadata with hybrid numbering

MIC3 contains many 3Cat chapters absent from TheTVDB. We therefore use TheTVDB Aired Order only for unique matches, retain the original 3Cat chapter number for every other episode, and write Plex NFO files as the canonical metadata source. Matroska tags duplicate the same values for portability but never replace the NFO contract; this avoids unsafe fuzzy renames while keeping the full collection visible.
