app-title = Now Plexing
app-comment = Watch your Plex Media Server's active playback sessions from the COSMIC panel
app-keywords = Plex;Media;Now Playing;Sessions;Streaming;

# Sessions view
nothing-playing = Nothing is playing
loading = Checking Plex…
unknown-user = Someone
open-settings = Settings

# Settings view
settings = Settings
back = Back
save = Save
plex-token = Plex token
plex-token-placeholder = X-Plex-Token
plex-token-help = Discovers your server automatically. Stored in your COSMIC configuration.

# Failure states
error-no-token = Add a Plex token in Settings
error-unauthorized = Plex rejected this token
error-no-server = No Plex Media Server found for this account
error-unreachable = Can't reach your Plex server
error-protocol = Plex returned something unexpected

# Alerts
alerts = Notify me
alerts-help = Send a desktop notification when the number of playing streams crosses the level below.
alert-level = Alert level
alert-level-help = Fires when this many streams or fewer are playing, and again when the count climbs back above it.
alert-quiet = { $count ->
    [0] Nobody is streaming
    [one] Only 1 stream is still playing
   *[other] Only { $count } streams are still playing
}
alert-busy = { $count ->
    [one] 1 stream is playing
   *[other] { $count } streams are playing
}
