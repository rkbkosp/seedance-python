# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "httpx>=0.27,<1",
# ]
# ///

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
from datetime import UTC, datetime
from typing import Any

import httpx


BILLING_ENDPOINT = "https://open.volcengineapi.com"
BILLING_HOST = "open.volcengineapi.com"
BILLING_SERVICE = "billing"
BILLING_REGION = "cn-beijing"
BILLING_VERSION = "2022-01-01"
ACTION = "QueryBalanceAcct"
PAYLOAD = "{}"


def sha256_hex(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def hmac_bytes(key: bytes, value: str) -> bytes:
    return hmac.new(key, value.encode("utf-8"), hashlib.sha256).digest()


def sign(secret_key: str, short_date: str) -> bytes:
    k_date = hmac_bytes(f"VOLC{secret_key}".encode("utf-8"), short_date)
    k_region = hmac_bytes(k_date, BILLING_REGION)
    k_service = hmac_bytes(k_region, BILLING_SERVICE)
    return hmac_bytes(k_service, "request")


def build_headers(access_key: str, secret_key: str, security_token: str | None) -> tuple[str, dict[str, str]]:
    now = datetime.now(UTC)
    amz_date = now.strftime("%Y%m%dT%H%M%SZ")
    short_date = now.strftime("%Y%m%d")
    payload_hash = sha256_hex(PAYLOAD)

    signed_headers = "content-type;host;x-content-sha256;x-date"
    extra_header = ""
    headers: dict[str, str] = {
        "Host": BILLING_HOST,
        "Content-Type": "application/json; charset=utf-8",
        "X-Date": amz_date,
        "X-Content-Sha256": payload_hash,
    }
    if security_token:
        signed_headers = "content-type;host;x-content-sha256;x-date;x-security-token"
        extra_header = f"x-security-token:{security_token}\n"
        headers["X-Security-Token"] = security_token

    canonical_query = f"Action={ACTION}&Version={BILLING_VERSION}"
    canonical_headers = (
        "content-type:application/json; charset=utf-8\n"
        f"host:{BILLING_HOST}\n"
        f"x-content-sha256:{payload_hash}\n"
        f"x-date:{amz_date}\n"
        f"{extra_header}"
    )
    canonical_request = (
        "POST\n/\n"
        f"{canonical_query}\n"
        f"{canonical_headers}\n"
        f"{signed_headers}\n"
        f"{payload_hash}"
    )
    credential_scope = f"{short_date}/{BILLING_REGION}/{BILLING_SERVICE}/request"
    string_to_sign = (
        "HMAC-SHA256\n"
        f"{amz_date}\n"
        f"{credential_scope}\n"
        f"{sha256_hex(canonical_request)}"
    )
    signature = hmac.new(sign(secret_key, short_date), string_to_sign.encode("utf-8"), hashlib.sha256).hexdigest()
    headers["Authorization"] = (
        f"HMAC-SHA256 Credential={access_key}/{credential_scope}, "
        f"SignedHeaders={signed_headers}, Signature={signature}"
    )
    return canonical_query, headers


def pretty(data: Any) -> str:
    return json.dumps(data, ensure_ascii=False, indent=2)


def main() -> int:
    parser = argparse.ArgumentParser(description="Probe Volcengine billing QueryBalanceAcct")
    parser.add_argument("--ak", default=os.getenv("VOLC_BILLING_AK"))
    parser.add_argument("--sk", default=os.getenv("VOLC_BILLING_SK"))
    parser.add_argument("--token", default=os.getenv("VOLC_BILLING_SECURITY_TOKEN"))
    args = parser.parse_args()

    if not args.ak or not args.sk:
        print("error: missing --ak / --sk (or VOLC_BILLING_AK / VOLC_BILLING_SK)")
        return 2

    canonical_query, headers = build_headers(args.ak, args.sk, args.token)
    url = f"{BILLING_ENDPOINT}/?{canonical_query}"

    print("request_url:", url)
    print("headers:")
    print(pretty(headers))

    with httpx.Client(timeout=30.0) as client:
        response = client.post(url, headers=headers, content=PAYLOAD)

    print("status_code:", response.status_code)
    print("response_text:")
    print(response.text)

    try:
        payload = response.json()
    except ValueError:
        return 1

    print("response_json:")
    print(pretty(payload))
    return 0 if response.is_success else 1


if __name__ == "__main__":
    raise SystemExit(main())
