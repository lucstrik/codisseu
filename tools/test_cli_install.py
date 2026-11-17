#!/usr/bin/env python3
"""Installer/command registration acceptance in disposable HOME directories."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class CommandInstall(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='codisseu-cli-install-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root/"Home ação 'quoted'"
        self.home.mkdir()
        self.bundle = self.root/'Bundle source'
        (self.bundle/'bin').mkdir(parents=True)
        (self.bundle/'share/codisseu/icons').mkdir(parents=True)
        (self.bundle/'share/codisseu/icons/256.png').write_bytes(b'owned installer fixture; not a screenshot')
        (self.bundle/'bin/codisseu').write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
        (self.bundle/'bin/codisseu').chmod(0o755)
        for source, destination in [('portable_install.sh','install.sh'), ('install_command_path.sh','install-command-path.sh')]:
            shutil.copyfile(ROOT/'tools'/source, self.bundle/destination)
        (self.bundle/'BUNDLE-ID').write_text('codisseu-0.1.0-linux-x86_64-123456abcdef\n')
        rows=[]
        for p in sorted(self.bundle.rglob('*')):
            if p.is_file(): rows.append(hashlib.sha256(p.read_bytes()).hexdigest()+'  '+p.relative_to(self.bundle).as_posix())
        (self.bundle/'MANIFEST.sha256').write_text('\n'.join(rows)+'\n')
        self.prefix = self.root/"Installed ' $(touch injected)"
        self.bins = self.home/'.local/bin'
        self.config = self.home/'Config space'
        self.data = self.home/'Data space'
        self.env = {**os.environ, 'HOME':str(self.home),'XDG_CONFIG_HOME':str(self.config),
                    'XDG_DATA_HOME':str(self.data),'CODISSEU_BIN_DIR':str(self.bins),
                    'PATH':'/usr/bin:/bin','SHELL':'/bin/bash'}
        self.env.pop('ZDOTDIR',None)

    def install(self, success=True):
        result=subprocess.run(['sh',str(self.bundle/'install.sh'),'--prefix',str(self.prefix)],
                              env=self.env,capture_output=True,text=True,timeout=20)
        if success: self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        else: self.assertNotEqual(result.returncode,0)
        return result

    def shell(self, command, rc='.bashrc'):
        return subprocess.run(['/bin/sh','-c','. "$HOME/'+rc+'"; '+command],
                              env=self.env,capture_output=True,text=True,timeout=15,check=True)

    def test_fresh_home_registers_command_for_new_shell_and_preserves_literal_install_path(self):
        self.install()
        result=self.shell('command -v codisseu; codisseu --version')
        self.assertEqual(result.stdout.splitlines(),[str(self.bins/'codisseu'),'--cli','--version'])
        self.assertEqual(self.shell('codisseu .').stdout.splitlines(),['--cli','.'])
        interactive=subprocess.run(['/bin/bash','--noprofile','--rcfile',str(self.home/'.bashrc'),'-ic','command -v codisseu; codisseu --help'],env=self.env,capture_output=True,text=True,check=True,timeout=15)
        self.assertEqual(interactive.stdout.splitlines(),[str(self.bins/'codisseu'),'--cli','--help'])
        self.assertFalse((self.root/'injected').exists())
        self.assertFalse((ROOT/'injected').exists())
        self.assertIn('Abra um novo terminal', self.install().stdout)

    def test_repeated_install_preserves_dotfiles_and_adds_one_managed_hook(self):
        original="export USER_OWN_SETTING='preserve me'\n# unrelated user configuration\n"
        (self.home/'.bashrc').write_text(original)
        (self.home/'.bashrc').chmod(0o600)
        self.install(); first=(self.home/'.bashrc').read_bytes()
        self.install()
        self.assertEqual((self.home/'.bashrc').read_bytes(),first)
        self.assertTrue(first.startswith(original.encode()))
        self.assertEqual(first.count(b'# >>> Codisseu command PATH >>>'),1)
        self.assertEqual((self.home/'.bashrc').stat().st_mode&0o777,0o600)
        self.assertEqual(self.shell('printf "%s" "$USER_OWN_SETTING"').stdout,'preserve me')
        self.assertEqual(self.shell('printf "%s" "$PATH"').stdout.split(':').count(str(self.bins)),1)

    def test_login_profile_and_zsh_config_are_preserved_and_registered(self):
        (self.home/'.bash_profile').write_text("export OWN_LOGIN='kept'\n")
        (self.home/'.zshrc').write_text('# own zsh config\n')
        self.install()
        self.assertEqual(self.shell('printf "%s\\n" "$OWN_LOGIN"; command -v codisseu','.bash_profile').stdout.splitlines(),['kept',str(self.bins/'codisseu')])
        self.assertTrue((self.home/'.zshrc').read_text().startswith('# own zsh config\n'))
        subprocess.run(['sh','-n',str(self.home/'.zshrc')],check=True)

    def test_existing_path_needs_no_dotfile_changes(self):
        self.env['PATH']=str(self.bins)+':/usr/bin:/bin'
        (self.home/'.bashrc').write_text('# untouched\n')
        self.install()
        self.assertEqual((self.home/'.bashrc').read_text(),'# untouched\n')
        self.assertFalse((self.config/'codisseu/path.sh').exists())
        self.assertEqual(subprocess.run(['codisseu','--help'],env=self.env,capture_output=True,text=True,check=True).stdout.splitlines(),['--cli','--help'])

    def test_foreign_launcher_is_preserved_before_copying_or_writing_profiles(self):
        self.bins.mkdir(parents=True)
        launcher=self.bins/'codisseu';launcher.write_text('foreign command\n')
        self.install(False)
        self.assertEqual(launcher.read_text(),'foreign command\n')
        self.assertFalse(self.prefix.exists())
        self.assertFalse((self.home/'.bashrc').exists())

    def test_symlink_and_unfinished_managed_block_do_not_destroy_shell_config(self):
        foreign=self.root/'foreign-rc';foreign.write_text('# preserve external file\n')
        rc=self.home/'.bashrc';rc.symlink_to(foreign)
        self.install(False)
        self.assertTrue(rc.is_symlink())
        self.assertEqual(foreign.read_text(),'# preserve external file\n')
        self.assertFalse(self.prefix.exists())
        rc.unlink();content='# >>> Codisseu command PATH >>>\n# preserve text after broken marker\n';rc.write_text(content)
        self.install(False)
        self.assertEqual(rc.read_text(),content)
        self.assertFalse(self.prefix.exists())

    def test_foreign_path_fragment_and_tampered_bundle_are_rejected(self):
        fragment=self.config/'codisseu/path.sh';fragment.parent.mkdir(parents=True)
        fragment.write_text('foreign fragment\n')
        self.install(False)
        self.assertEqual(fragment.read_text(),'foreign fragment\n')
        self.assertFalse(self.prefix.exists())
        fragment.unlink();(self.bundle/'bin/codisseu').write_text('tampered\n')
        self.install(False)
        self.assertFalse(self.prefix.exists())

    def test_installer_never_executes_user_startup_code(self):
        marker=self.root/'startup-executed'
        (self.home/'.bashrc').write_text('touch "'+str(marker)+'"\n')
        self.install()
        self.assertFalse(marker.exists())


if __name__=='__main__':unittest.main()
