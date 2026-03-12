# Corkscrew Privacy Policy

**Effective Date:** March 12, 2026
**Last Updated:** March 12, 2026

Corkscrew is an open-source mod manager for CrossOver/Wine games on macOS and Linux. Your privacy matters. This policy explains what data Corkscrew accesses, why, and what it does not do.

## Summary

- Corkscrew runs entirely on your computer. It has no backend servers.
- Corkscrew does not collect, store, or transmit any analytics, telemetry, or usage data.
- Corkscrew does not sell, share, or monetize your data in any way.
- All user data (mod lists, settings, chat history) is stored locally on your machine and never leaves it unless you explicitly use a cloud feature described below.

## Data Stored Locally

Corkscrew stores the following data on your local filesystem only:

- **Mod metadata**: Names, file paths, load orders, and configuration for your installed mods.
- **Settings and preferences**: Your app configuration, theme, and display preferences.
- **Chat history**: Conversations with the built-in AI assistant are stored locally in a SQLite database.
- **OAuth tokens**: If you sign in with Google (for Gemini AI), your access and refresh tokens are stored locally in a protected file (`~/.config/corkscrew/google_tokens.json` or equivalent) with restricted file permissions (mode 0600 on Unix systems). Tokens are never transmitted to anyone other than Google's OAuth and API servers.
- **NexusMods OAuth tokens**: If you sign in to NexusMods, your tokens are stored locally with the same protections.

You can delete all locally stored data at any time by removing the Corkscrew configuration directory.

## Cloud Services (Optional)

Corkscrew integrates with the following third-party services. **All cloud features are optional and opt-in.** Corkscrew functions fully offline without them.

### NexusMods API

- **What**: When you search for, browse, or download mods, Corkscrew communicates with the NexusMods API (nexusmods.com).
- **Data sent**: Search queries, mod IDs, and your NexusMods API key or OAuth token.
- **Purpose**: To search, browse, and download mods on your behalf.
- **Privacy policy**: [NexusMods Privacy Policy](https://help.nexusmods.com/article/117-privacy-policy)

### Google Gemini AI (via Google Sign-In)

- **What**: If you choose to sign in with Google, Corkscrew uses Google OAuth 2.0 to authenticate you and access the Gemini API for AI-assisted chat.
- **Data sent**: Your chat messages are sent to Google's Gemini API (`generativelanguage.googleapis.com`) for processing. Your Google account email and display name are retrieved for display in the app.
- **Data NOT sent**: Your mod lists, file paths, system information, or any other local data is not sent to Google unless you explicitly include it in a chat message.
- **Scopes requested**: `openid`, `userinfo.email`, `userinfo.profile`, `cloud-platform`, `generative-language.retriever`. The `cloud-platform` scope is required by Google for OAuth-based access to the Gemini API.
- **Revocation**: You can sign out at any time within Corkscrew, which revokes your token with Google and deletes all locally stored tokens. You can also revoke access from your [Google Account permissions page](https://myaccount.google.com/permissions).
- **Privacy policy**: [Google Privacy Policy](https://policies.google.com/privacy)

### Groq AI

- **What**: If you choose to use Groq as your cloud AI backend, chat messages are sent to Groq's API.
- **Data sent**: Chat messages and your Groq API key (which you provide).
- **Privacy policy**: [Groq Privacy Policy](https://groq.com/privacy-policy/)

### Cerebras AI

- **What**: If you choose to use Cerebras as your cloud AI backend, chat messages are sent to Cerebras's API.
- **Data sent**: Chat messages and your Cerebras API key (which you provide).
- **Privacy policy**: [Cerebras Privacy Policy](https://cerebras.ai/privacy-policy)

### Local AI (Ollama)

- **What**: If you use a local AI model via Ollama, all processing happens on your machine. No data is sent to any external server.

### Auto-Updater

- **What**: Corkscrew checks for updates by fetching a `latest.json` file from GitHub Releases.
- **Data sent**: A standard HTTPS request to `github.com`. No personally identifiable information is included.
- **Purpose**: To notify you of available updates and download them if you approve.

## Data We Do NOT Collect

- No analytics or telemetry
- No crash reports (crash logs are stored locally only)
- No device fingerprinting
- No IP address logging
- No advertising identifiers
- No cookies or tracking pixels
- No data shared with any third party beyond the explicit API calls described above

## Children's Privacy

Corkscrew does not knowingly collect any information from children under 13. The app does not require account creation and has no mechanism for collecting personal information from minors.

## Changes to This Policy

If this policy changes, the updated version will be published in the Corkscrew repository and the "Last Updated" date will be revised. Since Corkscrew has no way to contact users (we don't have your email), changes take effect when you update the app.

## Contact

For privacy questions or concerns, open an issue on the [Corkscrew GitHub repository](https://github.com/cashcon57/corkscrew/issues).

## Open Source

Corkscrew is open source under the MIT license. You can audit exactly what the app does at any time by reviewing the source code at [github.com/cashcon57/corkscrew](https://github.com/cashcon57/corkscrew).
