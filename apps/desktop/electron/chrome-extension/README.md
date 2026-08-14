# OpenTopia Chrome Bridge extension

This MV3 extension connects only the tab that a person explicitly approves. It
does not read Chrome's profile directory, copy cookies, or attach to every tab.

## Development

1. Open `chrome://extensions`, enable developer mode, and choose **Load
   unpacked**.
2. Select this directory.
3. Start OpenTopia, choose **Chrome** in the browser toolbar, and enter the
   one-time code in the extension popup.
4. Open the tab OpenTopia may use and click **连接当前标签页**.

The manifest key gives unpacked development builds a stable extension ID. The
loopback bridge rejects every other extension origin and still requires the
short-lived pairing code.

## Release distribution

Production builds should publish this directory through the Chrome Web Store
or an enterprise extension policy. Set `OPENTOPIA_CHROME_EXTENSION_ID` for the
desktop release to the ID assigned to that signed extension. The desktop bridge
advertises and accepts only that configured ID; the manifest's development ID
is only the fallback for local builds.

Do not add automatic profile copying or a shared Chrome `User Data` directory.
Existing login state is reused because commands run inside the user-approved
Chrome tab.
