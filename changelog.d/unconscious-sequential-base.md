- Fix repeated local environmental damage after Titanium's stored HP reaches
  its upper clamp. Apply each submitted delta to the previous stored base,
  retaining the raw HP receipt and cumulative submitted damage separately.
  Validate the host's stored-base count against the session's value before
  using it for unconscious posture.
