# OAuth 2.0 in brief

OAuth 2.0 is an authorization framework. It lets a client application obtain limited access to a protected resource on behalf of a user, without the client ever seeing the user's password.

Four roles take part. The resource owner is the user who owns the data. The client is the application that wants to use the data. The authorization server verifies the user and issues tokens. The resource server hosts the protected data and checks tokens on every request.

An access token is a credential the client presents to the resource server. It is short-lived, usually minutes to an hour, and carries a scope that limits what it can do. Because access tokens expire quickly, a refresh token is often issued alongside. The refresh token is long-lived and is sent only to the authorization server, never to the resource server, to obtain a new access token without asking the user again.

The authorization code flow is the recommended flow for web and mobile apps. The client redirects the user to the authorization server, the user signs in and consents, and the authorization server redirects back with a one-time authorization code. The client exchanges that code for tokens over a back channel. PKCE, which stands for Proof Key for Code Exchange, adds a per-request secret so that an intercepted code cannot be exchanged by an attacker. PKCE is required for public clients such as single-page and mobile apps.
