"""Ezerbot's adapter for workspace-mcp==2.1.1 (MIT, installed by uvx).

Only an access-token snapshot enters the server, never the refresh token or
OAuth client secret. The upstream public CredentialStore extension point keeps
credentials in memory instead of making a second on-disk token cache.
"""
import os
import sys
from google.oauth2.credentials import Credentials
from auth.credential_store import CredentialStore, set_credential_store


class EzerbotCredentials(CredentialStore):
    def __init__(self):
        self.email = os.environ.pop("GOOGLE_EMAIL")
        self.credentials = Credentials(
            token=os.environ.pop("ACCESS_TOKEN"),
            scopes=os.environ.pop("GOOGLE_SCOPES").split(),
        )

    def get_credential(self, user_email):
        if user_email != self.email:
            # Returning None would make upstream initiate an independent login.
            raise RuntimeError("Ce compte n’est pas connecté dans Ezerbot.")
        return self.credentials

    def store_credential(self, user_email, credentials):
        return False  # Ezerbot owns all token persistence/refresh.

    def delete_credential(self, user_email):
        return False

    def list_users(self):
        return [self.email]


set_credential_store(EzerbotCredentials())
# Also fail closed for automatic re-authentication inside an ordinary service
# tool. Only Ezerbot's approved broker may open a browser or mint fresh tokens.
import auth.google_auth as google_auth


async def authorization_owned_by_ezerbot(*args, **kwargs):
    return "Reconnectez ce service dans Paramètres › Connecteurs d’Ezerbot."


google_auth.start_auth_flow = authorization_owned_by_ezerbot
sys.argv += ["--disabled-tools", "start_google_auth"]
from main import main
main()
