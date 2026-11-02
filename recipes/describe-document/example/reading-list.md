# Reading list on OAuth

The best single explainer is the IETF overview at https://datatracker.ietf.org/doc/html/rfc6749 and the PKCE extension at https://datatracker.ietf.org/doc/html/rfc7636.

A diagram of the flow: ![flow](https://example.com/oauth-flow.png)

Key points: tokens are short-lived; refresh tokens must be stored safely; PKCE binds the authorization code to the client that asked for it.
