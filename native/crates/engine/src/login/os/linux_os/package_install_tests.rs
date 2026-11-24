//! The pure decisions of the Linux install-on-use: which package per
//! distribution, the unattended package manager calls, the elevated step's
//! names, and the German reasons of a declined or failed install.
use super::{attempt_from, family, message_of, packages, system_step, Component, Family, Manager};
use crate::login::os::shared::install_gate::Attempt;

fn ids(text: &str) -> Vec<String> {
    crate::screen::core::virtual_display_config::os_release_ids(text)
}

/// Every component has its package for every distribution family, and the
/// family follows os-release, else the package manager.
#[test]
fn package_install_task_packages_per_distribution() {
    let cases: [(&str, Option<Manager>, Option<Family>); 7] = [
        ("ID=ubuntu\nID_LIKE=debian\n", None, Some(Family::Debian)),
        ("ID=debian\n", Some(Manager::Dnf), Some(Family::Debian)),
        ("ID=fedora\n", None, Some(Family::Fedora)),
        (
            "ID=\"rhel\"\nID_LIKE=\"fedora\"\n",
            None,
            Some(Family::Fedora),
        ),
        ("ID=manjaro\nID_LIKE=arch\n", None, Some(Family::Arch)),
        (
            "ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n",
            None,
            Some(Family::Suse),
        ),
        ("ID=linuxmint-unknown\n", None, None),
    ];
    for (release, manager, expected) in cases {
        assert_eq!(family(&ids(release), manager), expected, "{release}");
    }
    // Case: an unknown distribution takes its package manager's family.
    for (manager, expected) in [
        (Manager::AptGet, Family::Debian),
        (Manager::Dnf, Family::Fedora),
        (Manager::Pacman, Family::Arch),
        (Manager::Zypper, Family::Suse),
    ] {
        assert_eq!(family(&ids("ID=other\n"), Some(manager)), Some(expected));
    }
    let table: [(Component, Family, &[&str]); 12] = [
        (Component::Fuse, Family::Debian, &["fuse3"]),
        (Component::Fuse, Family::Fedora, &["fuse3"]),
        (Component::Fuse, Family::Arch, &["fuse3"]),
        (Component::Fuse, Family::Suse, &["fuse3"]),
        (
            Component::FreeRdp,
            Family::Debian,
            &["freerdp3-sdl", "freerdp3-x11"],
        ),
        (Component::FreeRdp, Family::Fedora, &["freerdp"]),
        (Component::FreeRdp, Family::Arch, &["freerdp"]),
        (Component::FreeRdp, Family::Suse, &["freerdp"]),
        (Component::Dnsmasq, Family::Debian, &["dnsmasq-base"]),
        (Component::Dnsmasq, Family::Fedora, &["dnsmasq"]),
        (Component::Dnsmasq, Family::Arch, &["dnsmasq"]),
        (Component::Dnsmasq, Family::Suse, &["dnsmasq"]),
    ];
    for (component, family, expected) in table {
        assert_eq!(
            packages(component, family),
            expected,
            "{component:?} {family:?}"
        );
    }
    // Case: the package managers install without questions.
    assert_eq!(
        Manager::AptGet.install_args("fuse3"),
        [
            "-o",
            "DPkg::Lock::Timeout=300",
            "install",
            "-y",
            "--no-install-recommends",
            "fuse3"
        ]
    );
    assert_eq!(
        Manager::Dnf.install_args("freerdp"),
        ["install", "-y", "freerdp"]
    );
    assert_eq!(
        Manager::Pacman.install_args("dnsmasq"),
        ["-S", "--noconfirm", "--needed", "dnsmasq"]
    );
    assert_eq!(
        Manager::Zypper.install_args("fuse3"),
        ["--non-interactive", "install", "fuse3"]
    );
}

/// The elevated step: fixed names, an unknown component is refused before
/// anything runs, another step is not this module's.
#[test]
fn package_install_task_step_names() {
    for component in Component::ALL {
        let step = component.step();
        assert!(
            crate::rdp::host_setup::elevated::valid_step_name(&step),
            "{step}"
        );
        assert_eq!(
            Component::from_id(step.trim_start_matches(super::STEP_PREFIX)),
            Some(component)
        );
    }
    assert_eq!(Component::FreeRdp.step(), "package-install:freerdp");
    assert_eq!(
        system_step("package-install:curl"),
        Some((
            "package_state=failed:unknown_step".to_string(),
            super::exit::STEP
        ))
    );
    assert_eq!(system_step("virtual-display-install"), None);
    assert_eq!(system_step("install-xrdp"), None);
}

/// The answer of the pkexec call becomes the exact German reason: a
/// dismissed dialog is a decline, a missing agent or a failed package step
/// a failure with its message.
#[test]
fn package_install_task_decline_and_failure_reasons() {
    let packages = "Paket fuse3";
    let fuse = Component::Fuse;
    assert_eq!(
        attempt_from(fuse, packages, Some(0), "package_state=done\n", ""),
        Attempt::Installed
    );
    let declined = Attempt::Declined(
        "Die Installation von FUSE (Paket fuse3) wurde in der Systemabfrage abgelehnt.".into(),
    );
    assert_eq!(attempt_from(fuse, packages, Some(126), "", ""), declined);
    assert_eq!(
        attempt_from(
            fuse,
            packages,
            Some(127),
            "",
            "Error executing command as another user: Not authorized\n"
        ),
        declined
    );
    assert_eq!(
        attempt_from(
            fuse,
            packages,
            Some(127),
            "",
            "Error executing command as another user: No authentication agent found.\n"
        ),
        Attempt::Failed(
            "FUSE (Paket fuse3) kann nicht installiert werden: in dieser Sitzung läuft kein Polkit-Agent, der nach der Freigabe fragt.".into()
        )
    );
    assert_eq!(
        attempt_from(
            Component::FreeRdp,
            "Paket freerdp3-sdl oder freerdp3-x11",
            Some(60),
            "package_state=failed:package:freerdp3-x11: E: Unable to locate package freerdp3-x11\n",
            ""
        ),
        Attempt::Failed(
            "Die Paketverwaltung konnte FreeRDP 3 (Paket freerdp3-sdl oder freerdp3-x11) nicht installieren: freerdp3-x11: E: Unable to locate package freerdp3-x11".into()
        )
    );
    assert_eq!(
        attempt_from(
            Component::Dnsmasq,
            "Paket dnsmasq",
            Some(61),
            "package_state=failed:distribution_unknown\n",
            ""
        ),
        Attempt::Failed(
            "dnsmasq kann nicht installiert werden: auf diesem System gibt es weder apt-get, dnf, pacman noch zypper.".into()
        )
    );
    assert_eq!(
        attempt_from(fuse, packages, Some(1), "", "pkexec: crashed\n"),
        Attempt::Failed(
            "Die Installation von FUSE (Paket fuse3) endete mit Code 1: pkexec: crashed".into()
        )
    );
    // Case: the message is the last non-empty line, error output first.
    assert_eq!(
        message_of(
            "Reading package lists...\n",
            "W: x\nE: Unable to locate package y\n\n"
        ),
        "E: Unable to locate package y"
    );
    assert_eq!(
        message_of("error: target not found: freerdp\n", ""),
        "error: target not found: freerdp"
    );
}
