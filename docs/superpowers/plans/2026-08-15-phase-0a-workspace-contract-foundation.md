# Phase 0A Workspace and Contract Foundation Implementation Plan

> **SUPERSEDED - DO NOT EXECUTE**
>
> This document describes the retired Python/React/Hermes architecture. The
> canonical Rust design is [architecture.md](../../../architecture.md) and the
> active delivery roadmap is [phases.md](../../../phases.md).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish independently testable `backend/` and `frontend/` projects joined by one generated OpenAPI contract, then prove the boundary with a typed health endpoint, MSW-backed UI states, and real-process browser coverage.

**Architecture:** The backend is the sole source of API truth: strict Pydantic response models feed FastAPI's OpenAPI generator, and a deterministic exporter commits the canonical schema under `contracts/`. The frontend generates TypeScript types from that artifact, uses the same contract fixture through MSW, and reaches the backend through Vite's same-origin proxy. Phase 0A contains no trading logic, persistence, agents, live data, credentials, or remote access.

**Tech Stack:** Python 3.12+ with uv, FastAPI 0.139, Pydantic 2.13, pydantic-settings 2, Uvicorn 0.51, pytest 9, Ruff 0.15, and mypy 2; Node 22.13+ with npm 11, React 19.2, TypeScript 5.9, Vite 8, openapi-typescript 7.13, openapi-fetch 0.17, Vitest 4.1, Testing Library 16, MSW 2.15, Playwright 1.62, and axe-core 4.13.

## Global Constraints

- Work only in `backend/`, `frontend/`, `contracts/`, root developer tooling,
  and documentation named by this plan. Do not start Phase 0B or Phase 1 code.
- Require Python `>=3.12,<3.15`; use the local Python 3.13 interpreter when
  generating `backend/uv.lock`.
- Require Node `>=22.13,<23` and npm `>=11,<12`; commit
  `frontend/package-lock.json`.
- Bind all development servers to `127.0.0.1`. Do not add a public bind,
  authentication workaround, CORS wildcard, remote tunnel, or deployment file.
- Do not add SQLite, SQLAlchemy, Alembic, Hermes, Podman, OAuth, MCP, brokerage,
  market data, SSE, background jobs, or financial calculations in Phase 0A.
- Backend Pydantic models are the only hand-authored API schema. Never edit
  `contracts/openapi.json`, `frontend/src/api/generated/schema.ts`, or
  `frontend/src/api/generated/health-fixture.ts` by hand.
- Keep OpenAPI export deterministic: UTF-8, sorted object keys, compact
  separators, no ASCII escaping, no NaN, and exactly one trailing newline.
- The frontend may format backend fields, but it must not infer or recompute
  backend status. Its only Phase 0A compatibility rule is exact equality with
  health schema version `1.0`.
- Use only synthetic health data. Do not ingest credentials, account exports,
  portfolio information, symbols, market evidence, or personal data.
- Follow TDD for behavior: write a focused failing test, run it and verify the
  expected failure, add the minimum implementation, then rerun the focused and
  affected suites.
- Commit only after the focused red/green cycle and every check available at
  that task boundary passes.

---

## Authoritative References

- Delivery boundary and gates: [`phases.md`](../../../phases.md)
- System trust boundaries: [`architecture.md`](../../../architecture.md)
- Normative requirements:
  [`docs/superpowers/specs/2026-08-08-ai-stock-forum-design.md`](../specs/2026-08-08-ai-stock-forum-design.md)
- FastAPI exposes the generated schema through `app.openapi()`:
  [official FastAPI OpenAPI documentation](https://fastapi.tiangolo.com/how-to/extending-openapi/)
- Vite's supported React TypeScript template and Node floor:
  [official Vite guide](https://vite.dev/guide/)
- Type generation command:
  [official openapi-typescript CLI](https://openapi-ts.dev/cli)
- Deterministic in-process type generation:
  [official openapi-typescript Node API](https://openapi-ts.dev/node)
- Typed client construction:
  [official openapi-fetch API](https://openapi-ts.dev/openapi-fetch/api)
- Browser test installation and commands:
  [official Playwright guide](https://playwright.dev/docs/intro)

## Phase 0A Exit Criteria

- `make verify` passes the backend package build and unit/static checks,
  frontend unit/static/build checks, and all three generated-artifact drift
  checks.
- `make e2e` passes the same health-screen browser test once against MSW and
  once against the real FastAPI process through the Vite proxy.
- `GET /api/v1/health` returns exactly the versioned `HealthResponse` contract
  and makes no database, network, filesystem, clock, or environment read during
  request handling.
- Repeated OpenAPI exports are byte-identical. A backend contract change makes
  the backend snapshot test and frontend generation check fail until both
  generated artifacts are intentionally refreshed.
- The frontend has tested loading, connected, incompatible-contract, and
  unreachable-backend states.
- The production frontend bundle builds with mocks absent from emitted code,
  and a production-preview browser test observes no mock-worker request or
  service-worker registration.
- `make dev` starts the two development processes on loopback and the browser
  reaches the real backend through `/api` without CORS configuration.
- The repository contains no secret, live market/account fixture, broker-write
  surface, or Phase 0B/Phase 1 implementation.

## Phase 0A-Owned Additions at Phase Completion

The existing architecture, specification, planning, and repository-support
files remain in place. The tree below lists the files that Phase 0A owns or
modifies; it is not an instruction to delete authoritative documents.

```text
.gitignore
Makefile
README.md
architecture.md
phases.md
backend/
  .python-version
  pyproject.toml
  uv.lock
  src/
    ai_stock_forum/
      __init__.py
      api/
        __init__.py
        app.py
        contracts.py
        routes.py
        settings.py
      contracts/
        __init__.py
        export_openapi.py
  tests/
    api/
      test_health.py
      test_settings.py
    contracts/
      test_health_fixture.py
      test_openapi_export.py
contracts/
  README.md
  openapi.json
  fixtures/
    health/
      ok.json
frontend/
  .npmrc
  .prettierignore
  eslint.config.js
  index.html
  package.json
  package-lock.json
  playwright.config.ts
  playwright.mock.config.ts
  playwright.production.config.ts
  tsconfig.app.json
  tsconfig.json
  tsconfig.node.json
  vite.config.ts
  vitest.config.ts
  public/
    mockServiceWorker.js
  scripts/
    check-msw-worker.mjs
    generate-api.mjs
  e2e/
    health.spec.ts
    production.spec.ts
  src/
    App.tsx
    app.css
    main.tsx
    vite-env.d.ts
    api/
      client.ts
      contract.test.ts
      generated/
        health-fixture.ts
        schema.ts
    features/
      health/
        HealthPage.test.tsx
        HealthPage.tsx
        healthApi.test.ts
        healthApi.ts
    mocks/
      browser.ts
      handlers.ts
      server.ts
    test/
      setup.ts
docs/
  development.md
```

## Locked Public Interfaces

Backend:

```python
API_SCHEMA_VERSION: Final[str] = "1.0"
SERVICE_NAME: Final[Literal["ai-stock-forum-backend"]] = (
    "ai-stock-forum-backend"
)

class AppSettings(BaseSettings):
    environment: Literal["development", "test", "production"]
    host: Literal["127.0.0.1"]
    port: Literal[8000]

class HealthResponse(BaseModel):
    schema_version: str
    service: Literal["ai-stock-forum-backend"]
    status: Literal["ok"]
    application_version: str

def create_app(settings: AppSettings | None = None) -> FastAPI: ...
def canonical_openapi_bytes(app: FastAPI) -> bytes: ...
def write_openapi(output_path: Path, app: FastAPI | None = None) -> Path: ...
```

HTTP:

```text
GET /api/v1/health
200 application/json
{
  "schema_version": "1.0",
  "service": "ai-stock-forum-backend",
  "status": "ok",
  "application_version": "0.1.0"
}
```

Frontend:

```typescript
export const SUPPORTED_API_SCHEMA_VERSION = "1.0" as const;
export type ApiClient = Client<paths>;
export function createApiClient(baseUrl?: string): ApiClient;

export type HealthState =
  | { kind: "connected"; health: HealthResponse }
  | { kind: "incompatible"; receivedVersion: string }
  | { kind: "unreachable" };

export function checkHealth(client?: ApiClient): Promise<HealthState>;
```

The visible health page adds a local `loading` state while `checkHealth()` is
pending. `loading` is a React rendering concern and is deliberately absent from
the transport function's return type.

### Task 1: Bootstrap the backend and implement the typed health endpoint

**Files:**
- Create: `backend/.python-version`
- Create: `backend/pyproject.toml`
- Create: `backend/src/ai_stock_forum/__init__.py`
- Create: `backend/src/ai_stock_forum/api/__init__.py`
- Create: `backend/src/ai_stock_forum/api/app.py`
- Create: `backend/src/ai_stock_forum/api/contracts.py`
- Create: `backend/src/ai_stock_forum/api/routes.py`
- Create: `backend/src/ai_stock_forum/api/settings.py`
- Create: `backend/tests/api/test_health.py`
- Create: `backend/tests/api/test_settings.py`
- Create: `backend/uv.lock`

**Interfaces:**
- Consumes: no application code; Python 3.12+ and uv only.
- Produces: `AppSettings`, `HealthResponse`, `create_app()`, module-level `app`,
  `API_SCHEMA_VERSION`, `SERVICE_NAME`, and `GET /api/v1/health` for Tasks 2–6.

- [ ] **Step 1: Add the backend package metadata and the first failing tests**

Create `backend/.python-version`:

```text
3.13
```

Create `backend/pyproject.toml`:

```toml
[build-system]
requires = ["hatchling>=1.27,<2"]
build-backend = "hatchling.build"

[project]
name = "ai-stock-forum-backend"
version = "0.1.0"
description = "Local AI stock forum backend"
requires-python = ">=3.12,<3.15"
dependencies = [
  "fastapi>=0.139,<0.140",
  "pydantic>=2.13,<3",
  "pydantic-settings>=2.10,<3",
  "uvicorn[standard]>=0.51,<0.52",
]

[dependency-groups]
dev = [
  "httpx>=0.28,<0.29",
  "hypothesis>=6.165,<7",
  "mypy>=2,<3",
  "pytest>=9,<10",
  "pytest-cov>=7,<8",
  "ruff>=0.15,<0.16",
]

[tool.hatch.build.targets.wheel]
packages = ["src/ai_stock_forum"]

[tool.pytest.ini_options]
addopts = "-ra --strict-config --strict-markers"
testpaths = ["tests"]

[tool.coverage.run]
branch = true
source = ["ai_stock_forum"]

[tool.coverage.report]
fail_under = 95
show_missing = true
skip_covered = true

[tool.ruff]
line-length = 88
target-version = "py312"

[tool.ruff.lint]
select = ["E", "F", "I", "UP", "B", "SIM", "RUF"]

[tool.mypy]
python_version = "3.12"
strict = true
packages = ["ai_stock_forum"]
```

Create only the package marker needed for installation:

```python
# backend/src/ai_stock_forum/__init__.py
__version__ = "0.1.0"
```

Create `backend/tests/api/test_health.py`:

```python
from fastapi.testclient import TestClient

from ai_stock_forum.api.app import create_app


def test_health_returns_the_exact_versioned_contract() -> None:
    response = TestClient(create_app()).get("/api/v1/health")

    assert response.status_code == 200
    assert response.headers["content-type"] == "application/json"
    assert response.json() == {
        "schema_version": "1.0",
        "service": "ai-stock-forum-backend",
        "status": "ok",
        "application_version": "0.1.0",
    }


def test_create_app_retains_explicit_settings() -> None:
    from ai_stock_forum.api.settings import AppSettings

    settings = AppSettings(environment="test", _env_file=None)

    application = create_app(settings)

    assert application.state.settings is settings
```

Create `backend/tests/api/test_settings.py`:

```python
import pytest
from pydantic import ValidationError

from ai_stock_forum.api.settings import AppSettings


def test_settings_default_to_loopback() -> None:
    settings = AppSettings(_env_file=None)

    assert settings.environment == "development"
    assert settings.host == "127.0.0.1"
    assert settings.port == 8000


@pytest.mark.parametrize("port", [0, 8123, 65536])
def test_settings_reject_non_phase_zero_ports(port: int) -> None:
    with pytest.raises(ValidationError):
        AppSettings(port=port, _env_file=None)


def test_settings_reject_public_bind() -> None:
    with pytest.raises(ValidationError):
        AppSettings(host="0.0.0.0", _env_file=None)  # type: ignore[arg-type]
```

- [ ] **Step 2: Install the locked backend environment and verify the red state**

Run:

```bash
cd backend
uv sync --all-groups
uv run pytest tests/api/test_health.py tests/api/test_settings.py -v
```

Expected: collection fails with `ModuleNotFoundError: No module named
'ai_stock_forum.api'`. `uv sync` must create `backend/uv.lock`; a dependency
resolution failure is not the expected red state and must be resolved before
continuing.

- [ ] **Step 3: Implement settings and the strict health response model**

Create `backend/src/ai_stock_forum/api/__init__.py` as an empty package marker.

Create `backend/src/ai_stock_forum/api/settings.py`:

```python
from functools import lru_cache
from typing import Literal

from pydantic_settings import BaseSettings, SettingsConfigDict


class AppSettings(BaseSettings):
    model_config = SettingsConfigDict(
        env_prefix="ASF_",
        extra="ignore",
        frozen=True,
    )

    environment: Literal["development", "test", "production"] = "development"
    host: Literal["127.0.0.1"] = "127.0.0.1"
    port: Literal[8000] = 8000


@lru_cache(maxsize=1)
def get_settings() -> AppSettings:
    return AppSettings()
```

Phase 0A deliberately fixes the only valid port at `8000`, matching the Vite
proxy and every supported Uvicorn launcher. A configurable port requires one
shared launcher/proxy contract and is deferred rather than accepted and then
silently ignored.

Create `backend/src/ai_stock_forum/api/contracts.py`:

```python
from typing import Final, Literal

from pydantic import BaseModel, ConfigDict, Field

from ai_stock_forum import __version__

API_SCHEMA_VERSION: Final[str] = "1.0"
SERVICE_NAME: Final[Literal["ai-stock-forum-backend"]] = (
    "ai-stock-forum-backend"
)


class HealthResponse(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    schema_version: str = Field(pattern=r"^\d+\.\d+$")
    service: Literal["ai-stock-forum-backend"]
    status: Literal["ok"]
    application_version: str = Field(pattern=r"^\d+\.\d+\.\d+$")


def build_health_response() -> HealthResponse:
    return HealthResponse(
        schema_version=API_SCHEMA_VERSION,
        service=SERVICE_NAME,
        status="ok",
        application_version=__version__,
    )
```

- [ ] **Step 4: Implement the route and application factory**

Create `backend/src/ai_stock_forum/api/routes.py`:

```python
from fastapi import APIRouter

from ai_stock_forum.api.contracts import HealthResponse, build_health_response

router = APIRouter(prefix="/api/v1", tags=["system"])


@router.get(
    "/health",
    operation_id="getHealth",
    response_model=HealthResponse,
)
async def get_health() -> HealthResponse:
    return build_health_response()
```

Create `backend/src/ai_stock_forum/api/app.py`:

```python
from fastapi import FastAPI

from ai_stock_forum import __version__
from ai_stock_forum.api.routes import router
from ai_stock_forum.api.settings import AppSettings, get_settings


def create_app(settings: AppSettings | None = None) -> FastAPI:
    resolved_settings = settings or get_settings()
    application = FastAPI(
        title="AI Stock Forum API",
        version=__version__,
        openapi_url="/api/v1/openapi.json",
        docs_url="/api/docs",
        redoc_url=None,
    )
    application.state.settings = resolved_settings
    application.include_router(router)
    return application


app = create_app()
```

- [ ] **Step 5: Verify the backend behavior and static checks**

Run:

```bash
cd backend
uv run pytest tests/api -v
uv run ruff check .
uv run ruff format --check .
uv run mypy src
```

Expected: `7 passed`; Ruff, formatting, and mypy exit `0`.

- [ ] **Step 6: Commit the backend foundation**

```bash
git add backend/.python-version backend/pyproject.toml backend/uv.lock backend/src backend/tests/api
git commit -m "feat: add typed backend health endpoint"
```

### Task 2: Export deterministic OpenAPI and add the shared health fixture

**Files:**
- Create: `backend/src/ai_stock_forum/contracts/__init__.py`
- Create: `backend/src/ai_stock_forum/contracts/export_openapi.py`
- Modify: `backend/pyproject.toml`
- Create: `backend/tests/contracts/test_openapi_export.py`
- Create: `backend/tests/contracts/test_health_fixture.py`
- Create: `contracts/openapi.json` (generated)
- Create: `contracts/fixtures/health/ok.json`
- Create: `contracts/README.md`

**Interfaces:**
- Consumes: `create_app()` and `HealthResponse` from Task 1.
- Produces: `canonical_openapi_bytes(app)`, `write_openapi(output_path, app)`,
  the `asf-export-openapi` command, canonical `contracts/openapi.json`, and the
  valid shared health fixture consumed by Tasks 3–5.

- [ ] **Step 1: Write failing exporter and fixture tests**

Create `backend/tests/contracts/test_openapi_export.py`:

```python
import json
from pathlib import Path

from ai_stock_forum.api.app import create_app
from ai_stock_forum.contracts.export_openapi import (
    canonical_openapi_bytes,
    main,
    write_openapi,
)

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
COMMITTED_OPENAPI = REPOSITORY_ROOT / "contracts" / "openapi.json"


def test_canonical_openapi_is_stable_and_compact() -> None:
    first = canonical_openapi_bytes(create_app())
    second = canonical_openapi_bytes(create_app())

    assert first == second
    assert first.endswith(b"\n")
    assert b"\n\n" not in first
    assert json.loads(first)["paths"]["/api/v1/health"]["get"][
        "operationId"
    ] == "getHealth"


def test_committed_openapi_matches_the_application() -> None:
    assert COMMITTED_OPENAPI.read_bytes() == canonical_openapi_bytes(create_app())


def test_write_openapi_returns_the_resolved_output(tmp_path: Path) -> None:
    output = tmp_path / "openapi.json"

    result = write_openapi(output, create_app())

    assert result == output.resolve()
    assert output.read_bytes() == canonical_openapi_bytes(create_app())


def test_main_writes_the_requested_output(tmp_path: Path) -> None:
    output = tmp_path / "from-cli.json"

    assert main([str(output)]) == 0
    assert output.read_bytes() == canonical_openapi_bytes(create_app())
```

Create `backend/tests/contracts/test_health_fixture.py`:

```python
from pathlib import Path

from ai_stock_forum.api.contracts import HealthResponse, build_health_response

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
HEALTH_FIXTURE = REPOSITORY_ROOT / "contracts" / "fixtures" / "health" / "ok.json"


def test_shared_health_fixture_matches_the_backend_contract() -> None:
    fixture = HealthResponse.model_validate_json(HEALTH_FIXTURE.read_text())

    assert fixture == build_health_response()
```

- [ ] **Step 2: Run the focused tests and verify the red state**

Run:

```bash
cd backend
uv run pytest tests/contracts -v
```

Expected: collection fails because
`ai_stock_forum.contracts.export_openapi` does not exist. A pass or a failure
from unrelated backend code is not the expected red state.

- [ ] **Step 3: Implement the deterministic exporter**

Create an empty `backend/src/ai_stock_forum/contracts/__init__.py`.

Create `backend/src/ai_stock_forum/contracts/export_openapi.py`:

```python
import argparse
import json
from collections.abc import Sequence
from pathlib import Path

from fastapi import FastAPI

from ai_stock_forum.api.app import create_app

REPOSITORY_ROOT = Path(__file__).resolve().parents[4]
DEFAULT_OUTPUT = REPOSITORY_ROOT / "contracts" / "openapi.json"


def canonical_openapi_bytes(app: FastAPI) -> bytes:
    text = json.dumps(
        app.openapi(),
        ensure_ascii=False,
        allow_nan=False,
        sort_keys=True,
        separators=(",", ":"),
    )
    return f"{text}\n".encode()


def write_openapi(output_path: Path, app: FastAPI | None = None) -> Path:
    resolved_output = output_path.resolve()
    resolved_output.parent.mkdir(parents=True, exist_ok=True)
    resolved_output.write_bytes(canonical_openapi_bytes(app or create_app()))
    return resolved_output


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Export canonical OpenAPI JSON")
    parser.add_argument(
        "output",
        nargs="?",
        type=Path,
        default=DEFAULT_OUTPUT,
    )
    arguments = parser.parse_args(argv)
    write_openapi(arguments.output)
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through main()
    raise SystemExit(main())
```

Add this script under `[project.scripts]` in `backend/pyproject.toml`:

```toml
[project.scripts]
asf-export-openapi = "ai_stock_forum.contracts.export_openapi:main"
```

- [ ] **Step 4: Add the shared fixture and generate the contract artifact**

Create `contracts/fixtures/health/ok.json`:

```json
{
  "application_version": "0.1.0",
  "schema_version": "1.0",
  "service": "ai-stock-forum-backend",
  "status": "ok"
}
```

Run:

```bash
cd backend
uv sync --all-groups
uv run asf-export-openapi ../contracts/openapi.json
```

Expected: `contracts/openapi.json` is created as one compact JSON line plus one
trailing newline. It contains `/api/v1/health`, `getHealth`, and
`HealthResponse`.

Create `contracts/README.md`:

````markdown
# Generated API contract

`openapi.json` is generated from backend Pydantic/FastAPI models. Do not edit it
by hand. Regenerate it with:

```bash
cd backend
uv run asf-export-openapi ../contracts/openapi.json
```

Files under `fixtures/` are synthetic, versioned examples validated by backend
tests and consumed by frontend mocks. They must not contain credentials, account
data, portfolio details, live market data, or personal information.
````

- [ ] **Step 5: Verify schema determinism, fixture validity, and all backend checks**

Run:

```bash
cd backend
uv run pytest tests/contracts -v
uv run pytest -q
uv run pytest --cov=ai_stock_forum --cov-report=term-missing
uv run ruff check .
uv run ruff format --check .
uv run mypy src
```

Expected: all tests pass, coverage is at least `95%`, and every static check
exits `0`. Run the exporter a second time, then rerun the committed-artifact
test to prove the generated bytes remain identical:

```bash
uv run asf-export-openapi ../contracts/openapi.json
uv run pytest tests/contracts/test_openapi_export.py::test_committed_openapi_matches_the_application -v
```

- [ ] **Step 6: Commit the generated contract boundary**

```bash
git add backend/pyproject.toml backend/uv.lock backend/src/ai_stock_forum/contracts backend/tests/contracts contracts
git commit -m "feat: export deterministic API contract"
```

### Task 3: Bootstrap the frontend and generate the typed API client

**Files:**
- Create: `frontend/.npmrc`
- Create: `frontend/package.json`
- Create: `frontend/package-lock.json`
- Create: `frontend/.prettierignore`
- Create: `frontend/eslint.config.js`
- Create: `frontend/index.html`
- Create: `frontend/tsconfig.json`
- Create: `frontend/tsconfig.app.json`
- Create: `frontend/tsconfig.node.json`
- Create: `frontend/vite.config.ts`
- Create: `frontend/vitest.config.ts`
- Create: `frontend/src/vite-env.d.ts`
- Create: `frontend/src/main.tsx`
- Create: `frontend/src/App.tsx`
- Create: `frontend/src/app.css`
- Create: `frontend/src/test/setup.ts`
- Create: `frontend/scripts/generate-api.mjs`
- Create: `frontend/src/api/contract.test.ts`
- Create: `frontend/src/api/generated/schema.ts` (generated)
- Create: `frontend/src/api/generated/health-fixture.ts` (generated)
- Create: `frontend/src/api/client.ts`

**Interfaces:**
- Consumes: `contracts/openapi.json` and `GET /api/v1/health` from Task 2.
- Produces: generated `paths` and `components`, a generated fixture guarded by
  `satisfies HealthResponse`, `ApiClient`, `createApiClient(baseUrl?)`, and
  `apiClient` for Task 4. It also establishes all frontend test, lint,
  type-check, formatting, and build commands.

- [ ] **Step 1: Add the locked frontend package and compiler configuration**

Create `frontend/.npmrc` so an unsupported Node or npm version is a hard error
rather than a warning:

```ini
engine-strict=true
```

Create `frontend/package.json`:

```json
{
  "name": "ai-stock-forum-frontend",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "engines": {
    "node": ">=22.13 <23",
    "npm": ">=11 <12"
  },
  "scripts": {
    "build": "tsc -b && vite build",
    "check:api": "node scripts/generate-api.mjs --check",
    "dev:frontend": "vite --host 127.0.0.1 --port 5173 --strictPort",
    "dev:mock": "vite --mode mock --host 127.0.0.1 --port 5173 --strictPort",
    "format": "prettier --write .",
    "format:check": "prettier --check .",
    "generate:api": "node scripts/generate-api.mjs",
    "lint": "eslint .",
    "test": "vitest run",
    "test:watch": "vitest",
    "typecheck": "tsc -b --pretty false"
  },
  "dependencies": {
    "openapi-fetch": "0.17.0",
    "react": "19.2.8",
    "react-dom": "19.2.8"
  },
  "devDependencies": {
    "@axe-core/playwright": "4.13.0",
    "@eslint/js": "10.0.1",
    "@playwright/test": "1.62.1",
    "@testing-library/dom": "10.4.1",
    "@testing-library/jest-dom": "7.0.1",
    "@testing-library/react": "16.3.2",
    "@testing-library/user-event": "14.6.4",
    "@types/node": "22.20.1",
    "@types/react": "19.2.18",
    "@types/react-dom": "19.2.4",
    "@vitejs/plugin-react": "6.0.5",
    "concurrently": "10.0.5",
    "eslint": "10.8.1",
    "eslint-plugin-react-hooks": "7.1.1",
    "eslint-plugin-react-refresh": "0.5.4",
    "globals": "17.11.0",
    "jsdom": "29.1.1",
    "msw": "2.15.0",
    "openapi-typescript": "7.13.0",
    "prettier": "3.9.6",
    "typescript": "5.9.3",
    "typescript-eslint": "8.67.0",
    "vite": "8.2.1",
    "vitest": "4.1.10"
  }
}
```

Create `frontend/.prettierignore`:

```text
dist/
node_modules/
playwright-report/
test-results/
public/mockServiceWorker.js
src/api/generated/schema.ts
src/api/generated/health-fixture.ts
package-lock.json
```

Create `frontend/tsconfig.json`:

```json
{
  "files": [],
  "references": [
    { "path": "./tsconfig.app.json" },
    { "path": "./tsconfig.node.json" }
  ]
}
```

Create `frontend/tsconfig.app.json`:

```json
{
  "compilerOptions": {
    "tsBuildInfoFile": "./node_modules/.tmp/tsconfig.app.tsbuildinfo",
    "target": "ES2023",
    "useDefineForClassFields": true,
    "lib": ["ES2023", "DOM", "DOM.Iterable"],
    "types": ["vite/client"],
    "allowJs": false,
    "skipLibCheck": true,
    "esModuleInterop": true,
    "allowSyntheticDefaultImports": true,
    "strict": true,
    "forceConsistentCasingInFileNames": true,
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "allowImportingTsExtensions": true,
    "verbatimModuleSyntax": true,
    "moduleDetection": "force",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx",
    "noFallthroughCasesInSwitch": true,
    "noUncheckedIndexedAccess": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true
  },
  "include": ["src", "../contracts/fixtures/**/*.json"]
}
```

Create `frontend/tsconfig.node.json`:

```json
{
  "compilerOptions": {
    "tsBuildInfoFile": "./node_modules/.tmp/tsconfig.node.tsbuildinfo",
    "skipLibCheck": true,
    "target": "ES2023",
    "lib": ["ES2023"],
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "allowImportingTsExtensions": true,
    "verbatimModuleSyntax": true,
    "moduleDetection": "force",
    "types": ["node"],
    "strict": true,
    "noEmit": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true
  },
  "include": [
    "eslint.config.js",
    "playwright.config.ts",
    "playwright.mock.config.ts",
    "playwright.production.config.ts",
    "vite.config.ts",
    "vitest.config.ts"
  ]
}
```

- [ ] **Step 2: Add Vite, Vitest, ESLint, and the minimal React shell**

Create `frontend/vite.config.ts`:

```typescript
import { fileURLToPath, URL } from "node:url";

import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url));

export default defineConfig(({ mode }) => ({
  plugins: [react()],
  publicDir: mode === "mock" ? "public" : false,
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
    fs: { allow: [repositoryRoot] },
    proxy: {
      "/api": "http://127.0.0.1:8000",
      "/events": "http://127.0.0.1:8000",
    },
  },
}));
```

Create `frontend/vitest.config.ts`:

```typescript
import { defineConfig, mergeConfig } from "vitest/config";

import viteConfig from "./vite.config";

export default defineConfig((configEnvironment) =>
  mergeConfig(
    viteConfig(configEnvironment),
    defineConfig({
      test: {
        environment: "jsdom",
        setupFiles: ["./src/test/setup.ts"],
      },
    })
  )
);
```

Create `frontend/eslint.config.js`:

```javascript
import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist", "public/mockServiceWorker.js", "src/api/generated"] },
  js.configs.recommended,
  {
    files: ["eslint.config.js", "scripts/**/*.mjs"],
    languageOptions: { globals: globals.node },
  },
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": [
        "warn",
        { allowConstantExport: true }
      ],
    },
  }
);
```

Create `frontend/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>AI Stock Forum</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

Create `frontend/src/vite-env.d.ts`:

```typescript
/// <reference types="vite/client" />
```

Create `frontend/src/App.tsx`:

```tsx
export function App() {
  return (
    <main>
      <h1>AI Stock Forum</h1>
      <p>Checking the local backend connection…</p>
    </main>
  );
}
```

Create `frontend/src/main.tsx`:

```tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import "./app.css";

const rootElement = document.getElementById("root");

if (rootElement === null) {
  throw new Error("Missing #root application element");
}

createRoot(rootElement).render(
  <StrictMode>
    <App />
  </StrictMode>
);
```

Create `frontend/src/app.css`:

```css
:root {
  color: #172033;
  background: #f5f7fb;
  font-family: Inter, ui-sans-serif, system-ui, sans-serif;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
}

* {
  box-sizing: border-box;
}

body {
  margin: 0;
  min-width: 320px;
  min-height: 100vh;
}

main {
  width: min(48rem, calc(100% - 2rem));
  margin: 4rem auto;
}
```

Create `frontend/src/test/setup.ts`:

```typescript
import "@testing-library/jest-dom/vitest";
```

- [ ] **Step 3: Write the failing generated-contract type test**

Create `frontend/src/api/contract.test.ts`:

```typescript
import { describe, expect, it } from "vitest";

import { healthFixture } from "./generated/health-fixture";
import type { components, paths } from "./generated/schema";

type HealthResponse = components["schemas"]["HealthResponse"];
type HealthOperation = paths["/api/v1/health"]["get"];

describe("generated health contract", () => {
  it("types the exact shared health payload", () => {
    const health: HealthResponse = healthFixture;
    const operationExists: HealthOperation extends never ? false : true = true;

    expect(health.status).toBe("ok");
    expect(operationExists).toBe(true);
  });
});
```

Install dependencies and run the type checker:

```bash
cd frontend
node --version
npm --version
npm config get engine-strict
npm install
npm run typecheck
```

Expected: Node reports a version in `>=22.13,<23`, npm reports `11.x`, and npm
reports `true` for `engine-strict`. `npm install` creates `package-lock.json`;
type checking fails because the two files under `./generated/` do not exist. A
dependency or engine failure is not the expected red state; switch to the
project's supported Node 22/npm 11 toolchain before continuing.

- [ ] **Step 4: Generate the client types and create the API client factory**

Create `frontend/scripts/generate-api.mjs`:

```javascript
import { mkdir, readFile, writeFile } from "node:fs/promises";

import openapiTS, { astToString } from "openapi-typescript";

const contractUrl = new URL("../../contracts/openapi.json", import.meta.url);
const healthFixtureUrl = new URL(
  "../../contracts/fixtures/health/ok.json",
  import.meta.url
);
const outputDirectoryUrl = new URL("../src/api/generated/", import.meta.url);
const schemaOutputUrl = new URL(
  "../src/api/generated/schema.ts",
  import.meta.url
);
const fixtureOutputUrl = new URL(
  "../src/api/generated/health-fixture.ts",
  import.meta.url
);
const schemaHeader =
  "// Generated from contracts/openapi.json. Do not edit.\n\n";
const fixtureHeader =
  "// Generated from contracts/fixtures/health/ok.json. Do not edit.\n\n";

const ast = await openapiTS(contractUrl, { alphabetize: true });
const healthFixture = JSON.parse(await readFile(healthFixtureUrl, "utf8"));
const expectedSchema = `${schemaHeader}${astToString(ast)}`;
const expectedFixture = `${fixtureHeader}import type { components } from "./schema";\n\nexport const healthFixture = ${JSON.stringify(
  healthFixture,
  null,
  2
)} as const satisfies components["schemas"]["HealthResponse"];\n`;
const outputs = [
  [schemaOutputUrl, expectedSchema],
  [fixtureOutputUrl, expectedFixture],
];

if (process.argv.includes("--check")) {
  let stale = false;
  for (const [outputUrl, expected] of outputs) {
    const current = await readFile(outputUrl, "utf8").catch(() => "");
    stale ||= current !== expected;
  }
  if (stale) {
    console.error(
      "Generated frontend contract artifacts are stale. " +
        "Run: cd frontend && npm run generate:api"
    );
    process.exitCode = 1;
  }
} else {
  await mkdir(outputDirectoryUrl, { recursive: true });
  await Promise.all(
    outputs.map(([outputUrl, expected]) =>
      writeFile(outputUrl, expected, "utf8")
    )
  );
}
```

Run:

```bash
cd frontend
npm run generate:api
```

Expected: `frontend/src/api/generated/schema.ts` is generated from
`contracts/openapi.json` and contains `"/api/v1/health"`, `getHealth`, and
`HealthResponse`. `frontend/src/api/generated/health-fixture.ts` is generated
from the shared JSON and uses `satisfies HealthResponse`; an incompatible
fixture therefore fails TypeScript rather than being hidden behind a cast.

Create `frontend/src/api/client.ts`:

```typescript
import createClient, { type Client } from "openapi-fetch";

import type { components, paths } from "./generated/schema";

export const SUPPORTED_API_SCHEMA_VERSION = "1.0" as const;

export type HealthResponse = components["schemas"]["HealthResponse"];
export type ApiClient = Client<paths>;

export function createApiClient(
  baseUrl = globalThis.location?.origin ?? "http://127.0.0.1:5173"
): ApiClient {
  return createClient<paths>({
    baseUrl,
    fetch: (...args) => globalThis.fetch(...args),
  });
}

export const apiClient = createApiClient();
```

- [ ] **Step 5: Verify types, generated-contract use, lint, format, and build**

Run:

```bash
cd frontend
npm run typecheck
npm run test -- src/api/contract.test.ts
npm run lint
npm run format
npm run format:check
npm run build
npm run check:api
```

Expected: the focused test reports `1 passed`; all remaining commands exit `0`.
`npm run check:api` must leave both generated frontend contract files
unchanged.

- [ ] **Step 6: Commit the frontend contract consumer**

```bash
git add frontend
git commit -m "feat: generate typed frontend API client"
```

### Task 4: Implement health states and the MSW-backed health screen

**Files:**
- Create: `frontend/public/mockServiceWorker.js` (generated)
- Create: `frontend/src/mocks/handlers.ts`
- Create: `frontend/src/mocks/server.ts`
- Create: `frontend/src/mocks/browser.ts`
- Modify: `frontend/src/test/setup.ts`
- Create: `frontend/src/features/health/healthApi.test.ts`
- Create: `frontend/src/features/health/healthApi.ts`
- Create: `frontend/src/features/health/HealthPage.test.tsx`
- Create: `frontend/src/features/health/HealthPage.tsx`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/main.tsx`
- Modify: `frontend/src/app.css`
- Modify: `frontend/package.json` through `msw init`
- Modify: `frontend/package-lock.json` through `msw init`

**Interfaces:**
- Consumes: `ApiClient`, `apiClient`, `HealthResponse`, and
  `SUPPORTED_API_SCHEMA_VERSION` from Task 3; the shared health fixture from
  Task 2.
- Produces: `HealthState`, `checkHealth(client?)`, `HealthPage`, reusable MSW
  handlers, Node test server, and opt-in browser worker for Task 5.

- [ ] **Step 1: Add reusable MSW test infrastructure**

Create `frontend/src/mocks/handlers.ts`:

```typescript
import { http, HttpResponse } from "msw";

import { healthFixture } from "../api/generated/health-fixture";

export const healthOk = healthFixture;

export const handlers = [
  http.get("*/api/v1/health", () => HttpResponse.json(healthOk)),
];
```

Create `frontend/src/mocks/server.ts`:

```typescript
import { setupServer } from "msw/node";

import { handlers } from "./handlers";

export const server = setupServer(...handlers);
```

Replace `frontend/src/test/setup.ts` with:

```typescript
import "@testing-library/jest-dom/vitest";

import { cleanup } from "@testing-library/react";
import { afterAll, afterEach, beforeAll } from "vitest";

import { server } from "../mocks/server";

beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterEach(() => {
  cleanup();
  server.resetHandlers();
});
afterAll(() => server.close());
```

- [ ] **Step 2: Write failing transport-state tests**

Create `frontend/src/features/health/healthApi.test.ts`:

```typescript
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";

import { createApiClient } from "../../api/client";
import { healthOk } from "../../mocks/handlers";
import { server } from "../../mocks/server";
import { checkHealth } from "./healthApi";

const client = createApiClient("http://localhost");

describe("checkHealth", () => {
  it("returns the connected state for the supported contract", async () => {
    await expect(checkHealth(client)).resolves.toEqual({
      kind: "connected",
      health: healthOk,
    });
  });

  it("returns incompatible when the server contract version differs", async () => {
    server.use(
      http.get("*/api/v1/health", () =>
        HttpResponse.json({ ...healthOk, schema_version: "2.0" })
      )
    );

    await expect(checkHealth(client)).resolves.toEqual({
      kind: "incompatible",
      receivedVersion: "2.0",
    });
  });

  it("returns unreachable when fetch fails", async () => {
    server.use(
      http.get("*/api/v1/health", () => HttpResponse.error())
    );

    await expect(checkHealth(client)).resolves.toEqual({ kind: "unreachable" });
  });
});
```

Run:

```bash
cd frontend
npm run test -- src/features/health/healthApi.test.ts
```

Expected: collection fails with `Failed to resolve import "./healthApi"`.

- [ ] **Step 3: Implement the typed health transport states**

Create `frontend/src/features/health/healthApi.ts`. The client factory's
late-bound `fetch` wrapper is intentional: it lets MSW replace
`globalThis.fetch` after modules load while preserving the same client in real
browser mode.

```typescript
import {
  type ApiClient,
  apiClient,
  type HealthResponse,
  SUPPORTED_API_SCHEMA_VERSION,
} from "../../api/client";

export type HealthState =
  | { kind: "connected"; health: HealthResponse }
  | { kind: "incompatible"; receivedVersion: string }
  | { kind: "unreachable" };

export async function checkHealth(
  client: ApiClient = apiClient
): Promise<HealthState> {
  try {
    const { data, error, response } = await client.GET("/api/v1/health");

    if (!response.ok || error !== undefined || data === undefined) {
      return { kind: "unreachable" };
    }
    if (data.schema_version !== SUPPORTED_API_SCHEMA_VERSION) {
      return {
        kind: "incompatible",
        receivedVersion: data.schema_version,
      };
    }
    return { kind: "connected", health: data };
  } catch {
    return { kind: "unreachable" };
  }
}
```

Run:

```bash
cd frontend
npm run test -- src/features/health/healthApi.test.ts
```

Expected: `3 passed`.

- [ ] **Step 4: Write failing health-page behavior tests**

Create `frontend/src/features/health/HealthPage.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";

import { healthOk } from "../../mocks/handlers";
import { server } from "../../mocks/server";
import { HealthPage } from "./HealthPage";

describe("HealthPage", () => {
  it("moves from loading to the connected backend details", async () => {
    render(<HealthPage />);

    expect(screen.getByRole("status")).toHaveTextContent(
      "Checking the local backend"
    );
    expect(await screen.findByText("Backend connected")).toBeInTheDocument();
    expect(screen.getByText("ai-stock-forum-backend")).toBeInTheDocument();
    expect(screen.getByText("0.1.0")).toBeInTheDocument();
  });

  it("shows an incompatible-contract alert", async () => {
    server.use(
      http.get("*/api/v1/health", () =>
        HttpResponse.json({ ...healthOk, schema_version: "2.0" })
      )
    );

    render(<HealthPage />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Backend contract 2.0 is not supported"
    );
  });

  it("can retry after the backend becomes reachable", async () => {
    server.use(
      http.get("*/api/v1/health", () => HttpResponse.error())
    );
    const user = userEvent.setup();

    render(<HealthPage />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Local backend is unreachable"
    );
    server.use(
      http.get("*/api/v1/health", () => HttpResponse.json(healthOk))
    );
    await user.click(screen.getByRole("button", { name: "Retry connection" }));

    expect(await screen.findByText("Backend connected")).toBeInTheDocument();
  });
});
```

Run:

```bash
cd frontend
npm run test -- src/features/health/HealthPage.test.tsx
```

Expected: collection fails with `Failed to resolve import "./HealthPage"`.

- [ ] **Step 5: Implement the health page and connect it to the app**

Create `frontend/src/features/health/HealthPage.tsx`:

```tsx
import { useCallback, useEffect, useState } from "react";

import { type ApiClient, apiClient } from "../../api/client";
import { checkHealth, type HealthState } from "./healthApi";

type HealthViewState = { kind: "loading" } | HealthState;

interface HealthPageProps {
  client?: ApiClient;
}

export function HealthPage({ client = apiClient }: HealthPageProps) {
  const [state, setState] = useState<HealthViewState>({ kind: "loading" });

  const retry = useCallback(() => {
    setState({ kind: "loading" });
    void checkHealth(client).then(setState);
  }, [client]);

  useEffect(() => {
    let active = true;
    void checkHealth(client).then((result) => {
      if (active) {
        setState(result);
      }
    });
    return () => {
      active = false;
    };
  }, [client]);

  if (state.kind === "loading") {
    return <p role="status">Checking the local backend…</p>;
  }

  if (state.kind === "incompatible") {
    return (
      <section className="status-card status-card--error" role="alert">
        <h2>Backend contract mismatch</h2>
        <p>
          Backend contract {state.receivedVersion} is not supported by this
          frontend.
        </p>
      </section>
    );
  }

  if (state.kind === "unreachable") {
    return (
      <section className="status-card status-card--error" role="alert">
        <h2>Local backend is unreachable</h2>
        <p>Start the loopback backend, then retry this connection.</p>
        <button type="button" onClick={retry}>
          Retry connection
        </button>
      </section>
    );
  }

  return (
    <section className="status-card" aria-labelledby="backend-status-heading">
      <h2 id="backend-status-heading">Backend connected</h2>
      <dl>
        <div>
          <dt>Service</dt>
          <dd>{state.health.service}</dd>
        </div>
        <div>
          <dt>Application version</dt>
          <dd>{state.health.application_version}</dd>
        </div>
        <div>
          <dt>Contract version</dt>
          <dd>{state.health.schema_version}</dd>
        </div>
      </dl>
    </section>
  );
}
```

Replace `frontend/src/App.tsx` with:

```tsx
import { HealthPage } from "./features/health/HealthPage";

export function App() {
  return (
    <main>
      <header>
        <p className="eyebrow">Local research workspace</p>
        <h1>AI Stock Forum</h1>
        <p>Human-approved research and defined-risk trade planning.</p>
      </header>
      <HealthPage />
    </main>
  );
}
```

Append to `frontend/src/app.css`:

```css
header {
  margin-bottom: 2rem;
}

.eyebrow {
  color: #3856a3;
  font-size: 0.8rem;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.status-card {
  padding: 1.25rem;
  border: 1px solid #cad3e5;
  border-radius: 0.75rem;
  background: #ffffff;
  box-shadow: 0 0.5rem 1.5rem rgb(31 48 85 / 8%);
}

.status-card--error {
  border-color: #b54848;
}

.status-card dl div {
  display: grid;
  grid-template-columns: minmax(9rem, 1fr) 2fr;
  gap: 1rem;
  padding: 0.5rem 0;
}

.status-card dt {
  font-weight: 700;
}

.status-card dd {
  margin: 0;
  overflow-wrap: anywhere;
}

button {
  min-height: 2.75rem;
  padding: 0.5rem 0.9rem;
  border: 0;
  border-radius: 0.4rem;
  color: #ffffff;
  background: #274894;
  font: inherit;
  font-weight: 700;
  cursor: pointer;
}

button:focus-visible {
  outline: 3px solid #f0a83c;
  outline-offset: 2px;
}
```

- [ ] **Step 6: Enable explicit browser mock mode**

Create `frontend/src/mocks/browser.ts`:

```typescript
import { setupWorker } from "msw/browser";

import { handlers } from "./handlers";

export const worker = setupWorker(...handlers);
```

Replace `frontend/src/main.tsx` with:

```tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import "./app.css";

async function enableMocks(): Promise<void> {
  if (import.meta.env.MODE !== "mock") {
    return;
  }
  const { worker } = await import("./mocks/browser");
  await worker.start({ onUnhandledRequest: "bypass" });
}

async function renderApplication(): Promise<void> {
  await enableMocks();
  const rootElement = document.getElementById("root");
  if (rootElement === null) {
    throw new Error("Missing #root application element");
  }
  createRoot(rootElement).render(
    <StrictMode>
      <App />
    </StrictMode>
  );
}

void renderApplication();
```

Generate the official service-worker asset:

```bash
cd frontend
npx msw init public --save
```

Expected: `public/mockServiceWorker.js` is created and `package.json` records
`public` as the MSW worker directory. Do not edit the generated worker.

- [ ] **Step 7: Verify every frontend health state and production behavior**

Run:

```bash
cd frontend
npm run test -- src/features/health
npm run test
npm run typecheck
npm run lint
npm run format
npm run format:check
npm run build
npm run check:api
```

Expected: the two health files report `6 passed`; the complete unit suite,
types, lint, format, production build, and generated-client check all exit `0`.
Require that Vite did not copy the worker into the production output, then
search every emitted JavaScript asset and require no match:

```bash
test ! -e dist/mockServiceWorker.js
rg "mockServiceWorker|setupWorker" dist/assets
```

Expected: `rg` exits `1`. This static gate is reinforced by Task 5's
production-preview browser test, which observes requests and registrations at
runtime.

- [ ] **Step 8: Commit the mock-backed health experience**

```bash
git add frontend
git commit -m "feat: add contract-aware backend health screen"
```

### Task 5: Prove mock, real-process, and production integration in the browser

**Files:**
- Create: `frontend/e2e/health.spec.ts`
- Create: `frontend/e2e/production.spec.ts`
- Create: `frontend/playwright.mock.config.ts`
- Create: `frontend/playwright.config.ts`
- Create: `frontend/playwright.production.config.ts`
- Create: `frontend/scripts/check-msw-worker.mjs`
- Modify: `frontend/package.json`
- Modify: `frontend/package-lock.json`
- Modify: `.gitignore` (append only; preserve every existing rule)
- Create: `Makefile`

**Interfaces:**
- Consumes: `HealthPage`, mock browser worker, Vite proxy, FastAPI health route,
  and the backend/frontend verification commands from Tasks 1–4.
- Produces: `npm run e2e:mock`, `npm run e2e:real`,
  `npm run e2e:production`, `npm run check:msw`, `npm run dev:all`, root
  `make dev`, `make verify`, and `make e2e` used by Task 6 and later phases.

- [ ] **Step 1: Write the browser acceptance test**

Create `frontend/e2e/health.spec.ts`:

```typescript
import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test("shows a contract-compatible local backend", async ({ page }) => {
  await page.goto("/");

  await expect(
    page.getByRole("heading", { name: "AI Stock Forum" })
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Backend connected" })
  ).toBeVisible();
  await expect(page.getByText("ai-stock-forum-backend")).toBeVisible();
  await expect(page.getByText("0.1.0")).toBeVisible();

  const accessibility = await new AxeBuilder({ page }).analyze();
  expect(accessibility.violations).toEqual([]);
});
```

Run:

```bash
cd frontend
npm run e2e:mock
```

Expected: npm exits nonzero with `Missing script: "e2e:mock"`. This is the
expected infrastructure red state; do not weaken the browser assertions.

- [ ] **Step 2: Add isolated mock, real-backend, and production configurations**

Create `frontend/playwright.mock.config.ts`:

```typescript
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "health.spec.ts",
  fullyParallel: true,
  forbidOnly: true,
  retries: 0,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:5173",
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: "npm run dev:mock",
    url: "http://127.0.0.1:5173",
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
```

Create `frontend/playwright.config.ts`:

```typescript
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "health.spec.ts",
  fullyParallel: true,
  forbidOnly: true,
  retries: 0,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:5173",
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: [
    {
      command:
        "cd ../backend && uv run uvicorn ai_stock_forum.api.app:create_app --factory --host 127.0.0.1 --port 8000",
      url: "http://127.0.0.1:8000/api/v1/health",
      reuseExistingServer: false,
      timeout: 120_000,
    },
    {
      command: "npm run dev:frontend",
      url: "http://127.0.0.1:5173",
      reuseExistingServer: false,
      timeout: 120_000,
    },
  ],
});
```

Create `frontend/e2e/production.spec.ts`:

```typescript
import { expect, test } from "@playwright/test";

test("production never starts the mock service worker", async ({ page }) => {
  const mockWorkerRequests: string[] = [];
  const serviceWorkers: string[] = [];

  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/mockServiceWorker.js") {
      mockWorkerRequests.push(request.url());
    }
  });
  page.context().on("serviceworker", (worker) => {
    serviceWorkers.push(worker.url());
  });

  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "AI Stock Forum" })
  ).toBeVisible();
  await page.waitForLoadState("networkidle");

  const registrations = await page.evaluate(async () => {
    if (!("serviceWorker" in navigator)) {
      return [];
    }
    return (await navigator.serviceWorker.getRegistrations()).map(
      (registration) => registration.scope
    );
  });

  expect(mockWorkerRequests).toEqual([]);
  expect(serviceWorkers).toEqual([]);
  expect(registrations).toEqual([]);
});
```

Create `frontend/playwright.production.config.ts`:

```typescript
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "production.spec.ts",
  fullyParallel: true,
  forbidOnly: true,
  retries: 0,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: "npm run preview:production",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
```

Add these scripts to `frontend/package.json` without changing existing scripts:

```json
{
  "scripts": {
    "dev:all": "concurrently --kill-others-on-fail --names backend,frontend \"npm run dev:backend\" \"npm run dev:frontend\"",
    "dev:backend": "cd ../backend && uv run uvicorn ai_stock_forum.api.app:create_app --factory --host 127.0.0.1 --port 8000 --reload",
    "e2e:mock": "playwright test --config playwright.mock.config.ts",
    "e2e:production": "npm run build && playwright test --config playwright.production.config.ts",
    "e2e:real": "playwright test --config playwright.config.ts",
    "preview:production": "vite preview --host 127.0.0.1 --port 4173 --strictPort"
  }
}
```

Run `npm install` after editing `package.json` so `package-lock.json` records the
final scripts and package metadata consistently.

- [ ] **Step 3: Install Chromium and run the browser test against MSW**

Run:

```bash
cd frontend
npx playwright install chromium
npm run e2e:mock
```

Expected: one Chromium test passes. The Playwright-managed Vite process runs in
`mock` mode, and no backend process is required.

- [ ] **Step 4: Run the same browser test through the real API proxy**

Run:

```bash
cd frontend
npm run e2e:real
```

Expected: one Chromium test passes. Playwright starts FastAPI on
`127.0.0.1:8000`, starts Vite on `127.0.0.1:5173`, and the browser requests
`/api/v1/health` through Vite. No CORS header or public bind is added.

- [ ] **Step 5: Prove production mock isolation and worker-file drift**

Create `frontend/scripts/check-msw-worker.mjs`:

```javascript
import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const temporaryDirectory = await mkdtemp(
  join(tmpdir(), "ai-stock-forum-msw-")
);
const mswCli = fileURLToPath(
  new URL("../node_modules/msw/cli/index.js", import.meta.url)
);

try {
  await execFileAsync(
    process.execPath,
    [mswCli, "init", temporaryDirectory],
    { cwd: new URL("..", import.meta.url) }
  );
  const generated = await readFile(
    join(temporaryDirectory, "mockServiceWorker.js")
  );
  const committed = await readFile(
    new URL("../public/mockServiceWorker.js", import.meta.url)
  );

  if (!generated.equals(committed)) {
    throw new Error(
      "MSW worker is stale. Run: cd frontend && npx msw init public --save"
    );
  }
} finally {
  await rm(temporaryDirectory, { recursive: true, force: true });
}
```

Add the following script to `frontend/package.json`, then run `npm install` to
keep the lockfile synchronized:

```json
{
  "scripts": {
    "check:msw": "node scripts/check-msw-worker.mjs"
  }
}
```

Run:

```bash
cd frontend
npm run check:msw
npm run e2e:production
```

Expected: the generated worker byte-comparison exits `0`; one Chromium test
passes against the production preview; no request for `/mockServiceWorker.js`
and no service-worker registration is observed.

- [ ] **Step 6: Append root ignore rules and add orchestration commands**

The repository already has a tracked, comprehensive `.gitignore`. Append this
section only; do not replace, reorder, or remove any existing line:

```gitignore
# AI Stock Forum generated frontend artifacts
backend/htmlcov/
frontend/coverage/
frontend/dist/
frontend/node_modules/
frontend/playwright-report/
frontend/test-results/
*.tsbuildinfo
```

Create `Makefile` using literal tab indentation for every recipe:

```makefile
.PHONY: contracts dev e2e verify verify-backend verify-contracts verify-frontend

dev:
	cd frontend && npm run dev:all

contracts:
	cd backend && uv run asf-export-openapi ../contracts/openapi.json
	cd frontend && npm run generate:api

verify-backend:
	cd backend && uv run pytest --cov=ai_stock_forum --cov-report=term-missing
	cd backend && uv run ruff check .
	cd backend && uv run ruff format --check .
	cd backend && uv run mypy src
	cd backend && uv build

verify-frontend:
	cd frontend && npm run test
	cd frontend && npm run typecheck
	cd frontend && npm run lint
	cd frontend && npm run format:check
	cd frontend && npm run build
	cd frontend && test ! -e dist/mockServiceWorker.js
	cd frontend && test -d dist/assets
	cd frontend && { rg "mockServiceWorker|setupWorker" dist/assets; match_status=$$?; test $$match_status -eq 1; }

verify-contracts:
	cd backend && uv run pytest tests/contracts/test_openapi_export.py::test_committed_openapi_matches_the_application -v
	cd frontend && npm run check:api
	cd frontend && npm run check:msw

verify: verify-backend verify-frontend verify-contracts

e2e:
	cd frontend && npm run e2e:mock
	cd frontend && npm run e2e:real
	cd frontend && npm run e2e:production
```

- [ ] **Step 7: Verify the root commands and process cleanup**

Run:

```bash
make verify
make e2e
```

Expected: the backend package build and checks, frontend checks, all three
artifact drift checks, two health-flow Chromium tests, and one production
isolation Chromium test pass. After `make e2e`, confirm ports `4173`, `5173`,
and `8000` are no longer listening; Playwright must clean up every child
process.

Start the manual development command:

```bash
make dev
```

Expected: both named processes start on loopback. Open
`http://127.0.0.1:5173`, confirm `Backend connected`, then press `Ctrl-C` once.
Both child processes must terminate.

- [ ] **Step 8: Commit the integrated local development gate**

```bash
git add .gitignore Makefile frontend/package.json frontend/package-lock.json frontend/playwright.config.ts frontend/playwright.mock.config.ts frontend/playwright.production.config.ts frontend/scripts/check-msw-worker.mjs frontend/e2e
git commit -m "test: prove frontend backend contract integration"
```

### Task 6: Document and run the complete Phase 0A gate

**Files:**
- Modify: `README.md`
- Create: `docs/development.md`
- Verify: every Phase 0A file listed in the repository shape

**Interfaces:**
- Consumes: all commands and behavior from Tasks 1–5.
- Produces: the supported developer workflow and the verified Phase 0A handoff
  for the separate Phase 1 backend/frontend plans.

- [ ] **Step 1: Replace the design-only README with the runnable project entry point**

Replace `README.md` with:

````markdown
# AI Stock Forum

A local, human-approved research forum where specialized Hermes agents debate
stock and defined-risk options swing trades. Version 1 can recommend an exact
trade plan but cannot preview, stage, transmit, replace, or cancel an order.

## Current status

Phase 0A establishes separate backend and frontend projects joined by a
generated OpenAPI contract. The current runnable screen verifies the local
backend connection; trading, agents, live evidence, brokerage, and approval are
not implemented yet.

## Requirements

- Python 3.12–3.14
- uv
- Node 22.13+ (22.x)
- npm 11.x
- Chromium installed through Playwright for browser tests

## Start development

```bash
cd backend && uv sync --all-groups
cd ../frontend && npm install
cd ..
make dev
```

Open <http://127.0.0.1:5173>. Both servers bind to loopback. Press `Ctrl-C`
once to stop them.

## Verify Phase 0A

```bash
make verify
make e2e
```

See [development.md](docs/development.md) for focused commands and the generated
contract workflow.

## Design documents

- [Architecture](architecture.md)
- [Delivery phases](phases.md)
- [Normative design](docs/superpowers/specs/2026-08-08-ai-stock-forum-design.md)
````

- [ ] **Step 2: Document focused development and contract commands**

Create `docs/development.md`:

````markdown
# Local development

## Project boundary

`backend/` owns validation, API behavior, and generated OpenAPI. `frontend/`
owns rendering and browser interaction. `contracts/` contains generated API
artifacts and synthetic shared fixtures. Frontend code must not reproduce
backend business rules.

## Backend

```bash
cd backend
uv sync --all-groups
uv run pytest -q
uv run ruff check .
uv run ruff format --check .
uv run mypy src
uv build
uv run uvicorn ai_stock_forum.api.app:create_app \
  --factory --host 127.0.0.1 --port 8000 --reload
```

The health endpoint is <http://127.0.0.1:8000/api/v1/health>.

## Frontend

```bash
cd frontend
npm install
npm run test
npm run typecheck
npm run lint
npm run format:check
npm run build
npm run dev:frontend
```

Use `npm run dev:mock` to run the UI against MSW without starting FastAPI. Mock
mode is explicit and is excluded from the production build.

## Generated contract workflow

Edit backend Pydantic models, routes, or the shared fixture first, then
regenerate all frontend contract artifacts:

```bash
make contracts
make verify
```

Review changes to `contracts/openapi.json`,
`frontend/src/api/generated/schema.ts`, and
`frontend/src/api/generated/health-fixture.ts`. Commit them with the backend
model or fixture change. Never edit a generated file directly.

## Browser integration

Install the managed browser once:

```bash
cd frontend
npx playwright install chromium
```

Run the mock, real-process, and production-isolation gates:

```bash
make e2e
```

The real-process test reaches FastAPI through the Vite `/api` proxy. The
production-preview test proves that the mock service worker is neither
requested nor registered. Phase 0A does not require CORS and does not permit
public network binding.

## Scope and data safety

Phase 0A accepts only the synthetic health fixture under `contracts/fixtures/`.
Do not place credentials, account data, portfolio details, market data, or
personal information in fixtures, logs, browser storage, or generated
contracts.
````

- [ ] **Step 3: Run the complete reproducible verification gate**

From the repository root, run:

```bash
make verify
make e2e
git diff --check
```

Expected:

- Backend pytest reaches at least `95%` branch coverage and `uv build` creates
  the ignored wheel and source distribution successfully.
- Ruff check, Ruff format, and mypy exit `0`.
- Frontend Vitest, TypeScript, ESLint, Prettier, and Vite build exit `0`.
- OpenAPI export, TypeScript regeneration, and the generated MSW-worker check
  leave no drift.
- One mock Chromium test and one real-process Chromium test pass with no axe
  violations; one production-preview Chromium test observes no mock worker.
- `git diff --check` prints nothing and exits `0`.

- [ ] **Step 4: Audit Phase 0A scope and generated files**

Run:

```bash
rg -n "sqlalchemy|alembic|hermes|podman|oauth|mcp|schwab|broker" backend/pyproject.toml frontend/package.json
```

Expected: no matches and exit `1`.

Run:

```bash
rg -n "TODO|TBD|FIXME|NotImplemented|pass$" backend frontend contracts README.md docs/development.md \
  -g '!frontend/public/mockServiceWorker.js' \
  -g '!frontend/src/api/generated/health-fixture.ts' \
  -g '!frontend/src/api/generated/schema.ts' \
  -g '!contracts/openapi.json'
```

Expected: no matches and exit `1`.

Run:

```bash
find backend frontend contracts \
  \( -path 'backend/.venv' -o -path 'frontend/node_modules' -o -path 'frontend/dist' \) -prune \
  -o -type f \( -name '.env*' -o -name '*.pem' -o -name '*.key' \) -print
```

Expected: no output and exit `0` from `find`; Phase 0A does not require any
environment file, private key, or certificate.

Run:

```bash
git status --short
git diff --stat
```

Expected: only `README.md`, `docs/development.md`, and any intentional formatting
changes from this task are uncommitted. Generated contract files must be clean.

- [ ] **Step 5: Commit the verified Phase 0A developer handoff**

```bash
git add README.md docs/development.md
git commit -m "docs: document phase zero development workflow"
```

- [ ] **Step 6: Verify the committed phase state**

Run:

```bash
make verify
make e2e
git status --short --branch
git log -6 --oneline --decorate
```

Expected: every check and all three browser modes pass again; the worktree is
clean; the six Phase 0A commits are visible in task order.

## Phase 0A Completion Boundary

When all six tasks pass, Phase 0A is complete. The next planning actions are:

1. Rewrite the existing Phase 1 risk-core plan so backend paths live under
   `backend/` and shared frontend-facing fixtures live under `contracts/`.
2. Write a separate Phase 1 frontend plan for golden risk-result components.
3. Execute those two Phase 1 tracks in parallel against the committed contract
   boundary.
4. Continue Phase 0B independently; pause for user-assisted Podman installation
   and ChatGPT device authorization when required.

Do not add trading behavior, Hermes, live evidence, account data, or approval to
the Phase 0A commits.
