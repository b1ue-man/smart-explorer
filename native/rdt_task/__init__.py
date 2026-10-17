"""Stages of the one batch suite ``rechte-desktop-tresore-task`` (plan.md D3).

Entry: ``python native/test-rechte-desktop-tresore-task.py --job <JOB> --log-root <DIR>``; the job
and stage table lives in that entrypoint. Runs only on the remote runner (AGENTS.md).

Modules of suite part A (U39): ``common`` (report, expected list, development builds, fixtures,
generic stage, inventory, emulator), ``prepare`` (lock, audit, static checks, artifact rdt-prepare),
``licenses`` (INT-LIC), ``perm`` (S1), ``drive`` (S2), ``vault`` (S3). The expected signals are in
``expected_signals.txt``; later packages move their tests from its section
``[noch nicht umgesetzt]`` to ``[umgesetzt]``.

Dock points for suite part B (U41): the files ``screen.py``, ``desktop.py`` and ``launcher.py`` of
this package. The entrypoint imports each one when it exists and may use these names:

``STAGES``        ``{stage: fn(ctx, stage)}`` for the stages the entrypoint assigns to the module
                  (screen: screen-pair, screen-x11, screen-wayland, kde, screen-windows,
                  screen-android-device, screen-android-api30; desktop: desktop-linux-x11,
                  desktop-linux-wayland, kde, desktop-gnome, desktop-gui-harness, desktop-windows;
                  launcher: launcher-jvm, launcher-android-device, launcher-android-api30). A stage
                  without a function runs ``ctx.run_default(stage)``; while the module file is
                  missing, every expected entry of its stages is reported as ``not_run``.
``CHECKS``        ``{name: fn(ctx) -> (status, reason)}`` for ``art=check`` entries in any stage,
                  among them the S4/S5 signals of ``prepare:static``.
``FIXTURES``      ``{name: fn(fixtures, name) -> (ok, reason)}``; a provider calls
                  ``fixtures.provide(name, env=..., remove=..., args=...)``.
``DEVICE_SETUP``  ``fn(ctx, device)``: called in the Android device jobs once the APKs are installed
                  (appops, accessibility service, launcher role, profiles, widget binding).

Statuses are ``common.PASSED``, ``common.FAILED`` and ``common.NOT_RUN``; ``not_run`` always carries
its reason and never counts as passed.
"""
