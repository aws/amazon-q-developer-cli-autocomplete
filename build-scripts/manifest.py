from dataclasses import dataclass
from enum import Enum

from const import APPLE_TEAM_ID, APP_NAME, CLI_BINARY_NAME, PTY_BINARY_NAME, CHAT_BINARY_NAME


class CdSigningType(Enum):
    DMG = "dmg"
    APP = "app"
    IME = "ime"


@dataclass
class EmbeddedRequirement:
    path: str
    identifier: str


def manifest(
    name: str,
    identifier: str,
    entitlements: bool | None = None,
    embedded_requirements: list[EmbeddedRequirement] | None = None,
):
    m = {
        "type": "app",
        "os": "osx",
        "name": name,
        "outputs": [{"label": "macos", "path": name}],
        "app": {
            "identifier": identifier,
            "signing_requirements": {
                "certificate_type": "developerIDAppDistribution",
                "app_id_prefix": APPLE_TEAM_ID,
            },
        },
    }

    if entitlements:
        m["app"]["signing_args"] = {"entitlements_path": "SIGNING_METADATA/entitlements.plist"}

    if embedded_requirements:
        m["app"]["embedded_requirements"] = {
            req.path: {
                "identifier": req.identifier,
            }
            for req in embedded_requirements
        }

    return m


def app_manifest():
    return manifest(
        name=f"{APP_NAME}.app",
        identifier="com.amazon.codewhisperer",
        entitlements=True,
        embedded_requirements=[
            EmbeddedRequirement(
                path=f"Contents/MacOS/{CLI_BINARY_NAME}",
                identifier=f"com.amazon.{CLI_BINARY_NAME}",
            ),
            EmbeddedRequirement(
                path=f"Contents/MacOS/{PTY_BINARY_NAME}",
                identifier=f"com.amazon.{PTY_BINARY_NAME}",
            ),
            EmbeddedRequirement(
                path=f"Contents/MacOS/{CHAT_BINARY_NAME}",
                identifier=f"com.amazon.{CHAT_BINARY_NAME}",
            ),
        ],
    )


def dmg_manifest(name: str):
    return manifest(
        name=name,
        identifier="com.amazon.codewhisperer.installer",
    )


def ime_manifest():
    return manifest(
        name="CodeWhispererInputMethod.app",
        identifier="com.amazon.inputmethod.codewhisperer",
    )
