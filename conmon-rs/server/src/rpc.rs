use crate::{child::Child, console::Console, iostreams::IOStreams, version::Version, Server};
use anyhow::Context;
use capnp::{capability::Promise, Error};
use capnp_rpc::pry;
use conmon_common::conmon_capnp::conmon;
use log::debug;
use std::{fs, path::PathBuf, sync::Arc};

macro_rules! pry_err {
    ($x:expr) => {
        pry!($x.map_err(|e| Error::failed(format!("{:#}", e))))
    };
}

impl conmon::Server for Server {
    fn version(
        &mut self,
        _: conmon::VersionParams,
        mut results: conmon::VersionResults,
    ) -> Promise<(), capnp::Error> {
        debug!("Got a version request");
        let mut response = results.get().init_response();
        let version = Version::new();
        response.set_version(version.version());
        response.set_tag(version.tag());
        response.set_commit(version.commit());
        response.set_build_date(version.build_date());
        response.set_rust_version(version.rust_version());
        Promise::ok(())
    }

    fn create_container(
        &mut self,
        params: conmon::CreateContainerParams,
        mut results: conmon::CreateContainerResults,
    ) -> Promise<(), capnp::Error> {
        let req = pry!(pry!(params.get()).get_request());
        debug!(
            "Got a create container request for id {}",
            pry!(req.get_id())
        );

        let maybe_console = if req.get_terminal() {
            pry_err!(Console::new()).into()
        } else {
            pry_err!(pry_err!(IOStreams::new()).start());
            None
        };

        let pidfile = pry!(pidfile_from_params(&params));
        let args = pry_err!(self.generate_runtime_args(&params, &maybe_console, &pidfile));
        let child_reaper = Arc::clone(self.reaper());
        let status = pry_err!(child_reaper.create_child(self.config().runtime(), args));

        let id = pry!(req.get_id()).to_string();
        debug!("Status for container ID {} is {}", id, status);
        if let Some(console) = maybe_console {
            pry_err!(console
                .wait_connected()
                .context("wait for console socket connection"));
        }

        let pid = pry_err!(pry!(fs::read_to_string(pidfile)).parse::<i32>());
        let exit_paths = pry!(path_vec_from_text_list(pry!(req.get_exit_paths())));
        let child = Child::new(id, pid, exit_paths);
        pry_err!(child_reaper.watch_grandchild(&child));

        // register child with server
        self.children.insert(child.id.clone(), child);

        // TODO FIXME why convert?
        results.get().init_response().set_container_pid(pid as u32);
        Promise::ok(())
    }
}

fn pidfile_from_params(params: &conmon::CreateContainerParams) -> capnp::Result<PathBuf> {
    let mut pidfile_pathbuf = PathBuf::from(params.get()?.get_request()?.get_bundle_path()?);
    pidfile_pathbuf.push("pidfile");

    debug!("pidfile is {}", pidfile_pathbuf.display());
    Ok(pidfile_pathbuf)
}

fn path_vec_from_text_list(tl: capnp::text_list::Reader) -> Result<Vec<PathBuf>, capnp::Error> {
    tl.iter().map(|r| r.map(PathBuf::from)).collect()
}
