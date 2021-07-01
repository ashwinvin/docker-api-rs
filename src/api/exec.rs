//! Run new commands inside running containers.
//!
//! API Reference: <https://docs.docker.com/engine/api/v1.41/#tag/Exec>

use std::collections::HashMap;

use futures_util::{stream::Stream, TryFutureExt};
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    conn::{tty, Headers, Payload},
    errors::{Error, Result},
    Docker,
};

/// Interface for docker exec instance
///
/// [Api Reference](https://docs.docker.com/engine/api/v1.41/#tag/Exec)
pub struct Exec<'docker> {
    docker: &'docker Docker,
    id: String,
}

impl<'docker> Exec<'docker> {
    fn new<S>(docker: &'docker Docker, id: S) -> Self
    where
        S: Into<String>,
    {
        Exec {
            docker,
            id: id.into(),
        }
    }

    /// Creates a new exec instance that will be executed in a container with id == container_id
    ///
    /// [Api Reference](https://docs.docker.com/engine/api/v1.41/#operation/ContainerExec)
    pub async fn create<C>(
        docker: &'docker Docker,
        container_id: C,
        opts: &ExecContainerOpts,
    ) -> Result<Exec<'docker>>
    where
        C: AsRef<str>,
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Response {
            id: String,
        }

        let body: Body = opts.serialize()?.into();

        let id = docker
            .post_json(
                &format!("/containers/{}/exec", container_id.as_ref()),
                Payload::Json(body),
            )
            .await
            .map(|resp: Response| resp.id)?;

        Ok(Exec::new(docker, id))
    }

    // This exists for Container::exec()
    //
    // We need to combine `Exec::create` and `Exec::start` into one method because otherwise you
    // needlessly tie the Stream to the lifetime of `container_id` and `opts`. This is because
    // `Exec::create` is async so it must occur inside of the `async move` block. However, this
    // means that `container_id` and `opts` are both expected to be alive in the returned stream
    // because we can't do the work of creating an endpoint from `container_id` or serializing
    // `opts`. By doing this work outside of the stream, we get owned values that we can then move
    // into the stream and have the lifetimes work out as you would expect.
    //
    // Yes, it is sad that we can't do the easy method and thus have some duplicated code.
    pub(crate) fn create_and_start<C>(
        docker: &'docker Docker,
        container_id: C,
        opts: &ExecContainerOpts,
    ) -> impl Stream<Item = Result<tty::TtyChunk>> + Unpin + 'docker
    where
        C: AsRef<str>,
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Response {
            id: String,
        }

        // To not tie the lifetime of `opts` to the stream, we do the serializing work outside of
        // the stream. But for backwards compatability, we have to return the error inside of the
        // stream.
        let body_result = opts.serialize();

        // To not tie the lifetime of `container_id` to the stream, we convert it to an (owned)
        // endpoint outside of the stream.
        let container_endpoint = format!("/containers/{}/exec", container_id.as_ref());

        Box::pin(
            async move {
                // Bubble up the error inside the stream for backwards compatability
                let body: Body = body_result?.into();

                let exec_id = docker
                    .post_json(&container_endpoint, Payload::Json(body))
                    .await
                    .map(|resp: Response| resp.id)?;

                let stream = Box::pin(docker.stream_post(
                    format!("/exec/{}/start", exec_id),
                    Payload::Json("{}"),
                    Headers::none(),
                ));

                Ok(tty::decode(stream))
            }
            .try_flatten_stream(),
        )
    }

    /// Get a reference to a set of operations available to an already created exec instance.
    ///
    /// It's in callers responsibility to ensure that exec instance with specified id actually
    /// exists. Use [Exec::create](Exec::create) to ensure that the exec instance is created
    /// beforehand.
    pub async fn get<I>(docker: &'docker Docker, id: I) -> Exec<'docker>
    where
        I: Into<String>,
    {
        Exec::new(docker, id)
    }

    /// Starts this exec instance returning a multiplexed tty stream
    ///
    /// [Api Reference](https://docs.docker.com/engine/api/v1.41/#operation/ExecStart)
    pub fn start(&self) -> impl Stream<Item = Result<tty::TtyChunk>> + 'docker {
        // We must take ownership of the docker reference to not needlessly tie the stream to the
        // lifetime of `self`.
        let docker = self.docker;
        // We convert `self.id` into the (owned) endpoint outside of the stream to not needlessly
        // tie the stream to the lifetime of `self`.
        let endpoint = format!("/exec/{}/start", &self.id);
        Box::pin(
            async move {
                let stream =
                    Box::pin(docker.stream_post(endpoint, Payload::Json("{}"), Headers::none()));

                Ok(tty::decode(stream))
            }
            .try_flatten_stream(),
        )
    }

    /// Inspect this exec instance to aquire detailed information
    ///
    /// [Api Reference](https://docs.docker.com/engine/api/v1.41/#operation/ExecInpsect)
    pub async fn inspect(&self) -> Result<ExecDetails> {
        self.docker
            .get_json(&format!("/exec/{}/json", &self.id)[..])
            .await
    }

    /// Resize the TTY session used by an exec instance. This only works if the exec was created
    /// with `tty` enabled.
    ///
    /// [Api Reference](https://docs.docker.com/engine/api/v1.41/#operation/ExecResize)
    pub async fn resize(&self, opts: &ExecResizeOpts) -> Result<()> {
        let body: Body = opts.serialize()?.into();

        self.docker
            .post_json(
                &format!("/exec/{}/resize", &self.id)[..],
                Payload::Json(body),
            )
            .await
    }
}

#[derive(Serialize, Debug)]
pub struct ExecContainerOpts {
    params: HashMap<&'static str, Vec<String>>,
    params_str: HashMap<&'static str, String>,
    params_bool: HashMap<&'static str, bool>,
}

impl ExecContainerOpts {
    /// return a new instance of a builder for Opts
    pub fn builder() -> ExecContainerOptsBuilder {
        ExecContainerOptsBuilder::default()
    }

    pub fn serialize(&self) -> Result<String> {
        let mut body = serde_json::Map::new();

        for (k, v) in &self.params {
            body.insert(
                (*k).to_owned(),
                serde_json::to_value(v).map_err(Error::SerdeJsonError)?,
            );
        }

        for (k, v) in &self.params_str {
            body.insert(
                (*k).to_owned(),
                serde_json::to_value(v).map_err(Error::SerdeJsonError)?,
            );
        }

        for (k, v) in &self.params_bool {
            body.insert(
                (*k).to_owned(),
                serde_json::to_value(v).map_err(Error::SerdeJsonError)?,
            );
        }

        serde_json::to_string(&body).map_err(Error::from)
    }
}

#[derive(Default)]
pub struct ExecContainerOptsBuilder {
    params: HashMap<&'static str, Vec<String>>,
    params_str: HashMap<&'static str, String>,
    params_bool: HashMap<&'static str, bool>,
}

impl ExecContainerOptsBuilder {
    /// Command to run, as an array of strings
    pub fn cmd<I, S>(&mut self, cmds: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        cmds.into_iter().for_each(|cmd| {
            self.params
                .entry("Cmd")
                .or_insert_with(Vec::new)
                .push(cmd.into());
        });
        self
    }

    /// A list of environment variables in the form "VAR=value"
    pub fn env<I, S>(&mut self, envs: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        envs.into_iter().for_each(|env| {
            self.params
                .entry("Env")
                .or_insert_with(Vec::new)
                .push(env.into());
        });

        self
    }

    /// Attach to stdout of the exec command
    pub fn attach_stdout(&mut self, stdout: bool) -> &mut Self {
        self.params_bool.insert("AttachStdout", stdout);
        self
    }

    /// Attach to stderr of the exec command
    pub fn attach_stderr(&mut self, stderr: bool) -> &mut Self {
        self.params_bool.insert("AttachStderr", stderr);
        self
    }

    /// Override the key sequence for detaching a container. Format is a single
    /// character [a-Z] or ctrl-<value> where <value> is one of: a-z, @, ^, [, , or _.
    pub fn detach_keys<S>(&mut self, format: S) -> &mut Self
    where
        S: Into<String>,
    {
        self.params_str.insert("DetachKeys", format.into());
        self
    }

    /// Allocate a pseudo-TTY
    pub fn tty(&mut self, allocate: bool) -> &mut Self {
        self.params_bool.insert("Tty", allocate);
        self
    }

    /// Runs the exec process with extended privileges. (Default: `false`)
    pub fn privileged(&mut self, privileged: bool) -> &mut Self {
        self.params_bool.insert("Privileged", privileged);
        self
    }

    pub fn user<S>(&mut self, user: S) -> &mut Self
    where
        S: Into<String>,
    {
        self.params_str.insert("User", user.into());
        self
    }

    pub fn working_dir<S>(&mut self, working_dir: S) -> &mut Self
    where
        S: Into<String>,
    {
        self.params_str.insert("WorkingDir", working_dir.into());
        self
    }

    pub fn build(&self) -> ExecContainerOpts {
        ExecContainerOpts {
            params: self.params.clone(),
            params_str: self.params_str.clone(),
            params_bool: self.params_bool.clone(),
        }
    }
}

/// Interface for creating volumes
#[derive(Serialize, Debug)]
pub struct ExecResizeOpts {
    params: HashMap<&'static str, Value>,
}

impl ExecResizeOpts {
    /// serialize Opts as a string. returns None if no Opts are defined
    pub fn serialize(&self) -> Result<String> {
        serde_json::to_string(&self.params).map_err(Error::from)
    }

    /// return a new instance of a builder for Opts
    pub fn builder() -> ExecResizeOptsBuilder {
        ExecResizeOptsBuilder::new()
    }
}

#[derive(Default)]
pub struct ExecResizeOptsBuilder {
    params: HashMap<&'static str, Value>,
}

impl ExecResizeOptsBuilder {
    pub(crate) fn new() -> Self {
        let params = HashMap::new();
        ExecResizeOptsBuilder { params }
    }

    pub fn height(&mut self, height: u64) -> &mut Self {
        self.params.insert("Name", json!(height));
        self
    }

    pub fn width(&mut self, width: u64) -> &mut Self {
        self.params.insert("Name", json!(width));
        self
    }

    pub fn build(&self) -> ExecResizeOpts {
        ExecResizeOpts {
            params: self.params.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ExecDetails {
    pub can_remove: bool,
    #[serde(rename = "ContainerID")]
    pub container_id: String,
    pub detach_keys: String,
    pub exit_code: Option<u64>,
    #[serde(rename = "ID")]
    pub id: String,
    pub open_stderr: bool,
    pub open_stdin: bool,
    pub open_stdout: bool,
    pub process_config: ProcessConfig,
    pub running: bool,
    pub pid: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessConfig {
    pub arguments: Vec<String>,
    pub entrypoint: String,
    pub privileged: bool,
    pub tty: bool,
    pub user: Option<String>,
}
