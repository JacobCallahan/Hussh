.PHONY: setup-dev

setup-dev:
	sudo dnf install -y gcc perl-core openssl-devel pkgconf-pkg-config
	test -d .venv || uv venv .venv
	uv pip install --python .venv/bin/python -e '.[dev]'
	@echo "Development environment ready. Activate it with: source .venv/bin/activate"
