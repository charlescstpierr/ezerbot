"""Offline fake workspace-mcp boundary: no installed provider package or network."""
import abc
import asyncio
import os
import runpy
import sys
import types


def module(name):
    result = types.ModuleType(name)
    result.__path__ = []
    sys.modules[name] = result
    return result


module("google")
module("google.oauth2")
credentials = module("google.oauth2.credentials")


class Credentials:
    def __init__(self, *, token, scopes):
        assert token == "fake-google-access-token"
        assert scopes == ["openid", "email", "gmail.modify"]
        self.token = token


credentials.Credentials = Credentials
module("auth")
google_auth = module("auth.google_auth")
store = module("auth.credential_store")


class CredentialStore(abc.ABC):
    @abc.abstractmethod
    def get_credential(self, user_email): pass
    @abc.abstractmethod
    def store_credential(self, user_email, credentials): pass
    @abc.abstractmethod
    def delete_credential(self, user_email): pass
    @abc.abstractmethod
    def list_users(self): pass


store.CredentialStore = CredentialStore
store.set_credential_store = lambda value: setattr(store, "active", value)
main = module("main")


def fake_server_main():
    active = store.active
    assert active.list_users() == ["factice@example.test"]
    assert active.get_credential("factice@example.test").token == "fake-google-access-token"
    try:
        active.get_credential("autre@example.test")
        raise AssertionError("another account must not initiate independent OAuth")
    except RuntimeError as error:
        assert "Ezerbot" in str(error)
    assert not active.store_credential("factice@example.test", None)
    assert not active.delete_credential("factice@example.test")
    assert "ACCESS_TOKEN" not in os.environ
    assert "GOOGLE_EMAIL" not in os.environ
    assert "GOOGLE_SCOPES" not in os.environ
    assert "REFRESH_TOKEN" not in os.environ
    assert "CLIENT_SECRET" not in os.environ
    assert "Paramètres" in asyncio.run(google_auth.start_auth_flow("autre@example.test"))
    assert sys.argv[-2:] == ["--disabled-tools", "start_google_auth"]
    print("memory-only credential adapter: ok")


main.main = fake_server_main
bridge = sys.argv[1]
runpy.run_path(bridge, run_name="__main__")
