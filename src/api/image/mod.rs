//! Create and manage images.
pub mod models;
pub mod opts;

pub use models::*;
pub use opts::*;

use std::io::Read;

use futures_util::{stream::Stream, TryFutureExt, TryStreamExt};

use crate::{
    conn::{Headers, Payload, AUTH_HEADER},
    util::{
        tarball,
        url::{construct_ep, encoded_pair, encoded_pairs},
    },
    Result,
};

impl_api_ty!(Image => name);

pub type DeleteStatus = Vec<Status>;

impl<'docker> Image<'docker> {
    impl_api_ep! {img: Image, resp
        Inspect -> &format!("/images/{}/json", img.name), Details
        DeleteWithOpts -> &format!("/images/{}", img.name), DeleteStatus, delete_json
    }

    api_doc! { Image => History
    /// Lists the history of the images set of changes.
    |
    pub async fn history(&self) -> Result<Vec<History>> {
        self.docker
            .get_json(&format!("/images/{}/history", self.name))
            .await
    }}

    api_doc! { Image => Get
    /// Export this image to a tarball.
    |
    pub fn export(&self) -> impl Stream<Item = Result<Vec<u8>>> + Unpin + 'docker {
        Box::pin(
            self.docker
                .stream_get(format!("/images/{}/get", self.name))
                .map_ok(|c| c.to_vec()),
        )
    }}

    api_doc! { Image => Tag
    /// Adds a tag to an image.
    |
    pub async fn tag(&self, opts: &TagOpts) -> Result<()> {
        let ep = construct_ep(format!("/images/{}/tag", self.name), opts.serialize());
        self.docker.post(&ep, Payload::empty()).await.map(|_| ())
    }}

    api_doc! { Image => Push
    /// Push an image to registry.
    |
    pub async fn push(&self, opts: &ImagePushOpts) -> Result<()> {
        let ep = construct_ep(format!("/images/{}/push", self.name), opts.serialize());

        let headers = opts
            .auth_header()
            .map(|auth| Headers::single(AUTH_HEADER, auth))
            .unwrap_or_else(Headers::default);

        self.docker
            .post_headers(&ep, Payload::empty(), headers)
            .await
            .map(|_| ())
    }}

    api_doc! { Distribution => Inspect
    /// Return image digest and platform information by contacting the registry.
    |
    pub async fn distribution_inspect(&self) -> Result<DistributionInspectInfo> {
        self.docker
            .post_json(
                &format!("/distribution/{}/json", self.name),
                Payload::empty(),
            )
            .await
    }}
}

impl<'docker> Images<'docker> {
    impl_api_ep! {img: Image, resp
        List -> "/images/json"
        Prune ->  "/images/prune"
    }

    api_doc! { Image => Build
    /// Builds a new image build by reading a Dockerfile in a target directory.
    |
    pub fn build(
        &self,
        opts: &BuildOpts,
    ) -> impl Stream<Item = Result<ImageBuildChunk>> + Unpin + 'docker {
        let ep = construct_ep("/build", opts.serialize());

        // To not tie the lifetime of `opts` to the 'stream, we do the tarring work outside of the
        // stream. But for backwards compatability, we have to return the error inside of the
        // stream.
        let mut bytes = Vec::default();
        let tar_result = tarball::dir(&mut bytes, &opts.path);

        // We must take ownership of the Docker reference. If we don't then the lifetime of 'stream
        // is incorrectly tied to `self`.
        let docker = self.docker;
        Box::pin(
            async move {
                // Bubble up error inside the stream for backwards compatability
                tar_result?;

                let value_stream =
                    docker.stream_post_into(ep, Payload::Tar(bytes), Headers::none());

                Ok(value_stream)
            }
            .try_flatten_stream(),
        )
    }}

    api_doc! { Image => Search
    /// Search for docker images by term.
    |
    pub async fn search<T>(&self, term: T) -> Result<Vec<SearchResult>>
    where
        T: AsRef<str>,
    {
        self.docker
            .get_json(&construct_ep("/images/search", Some(encoded_pair("term", term.as_ref()))))
            .await
    }}

    api_doc! { Image => Pull
    /// Pull and create a new docker images from an existing image.
    |
    pub fn pull(
        &self,
        opts: &PullOpts,
    ) -> impl Stream<Item = Result<ImageBuildChunk>> + Unpin + 'docker {
        let headers = opts
            .auth_header()
            .map(|a| Headers::single(AUTH_HEADER, a));

        Box::pin(self.docker.stream_post_into(
            construct_ep("/images/create", opts.serialize()),
            Payload::empty(),
            headers,
        ))
    }}

    api_doc! { Image => GetAll
    /// Exports a collection of named images,
    /// either by name, name:tag, or image id, into a tarball.
    |
    pub fn export(&self, names: Vec<&str>) -> impl Stream<Item = Result<Vec<u8>>> + 'docker {
        self.docker
            .stream_get(format!(
                "/images/get?{}",
                encoded_pairs(names.iter().map(|n| ("names", *n)))
            ))
            .map_ok(|c| c.to_vec())
    }}

    api_doc! { Image => Load
    /// Imports an image or set of images from a given tarball source.
    /// Source can be uncompressed on compressed via gzip, bzip2 or xz.
    |
    pub fn import<R>(
        self,
        mut tarball: R,
    ) -> impl Stream<Item = Result<ImageBuildChunk>> + Unpin + 'docker
    where
        R: Read + Send + 'docker,
    {
        Box::pin(
            async move {
                let mut bytes = Vec::default();

                tarball.read_to_end(&mut bytes)?;

                let value_stream = self.docker.stream_post_into(
                    "/images/load",
                    Payload::Tar(bytes),
                    Headers::none(),
                );
                Ok(value_stream)
            }
            .try_flatten_stream(),
        )
    }}

    api_doc! { Image => Push
    /// Push an image to registry.
    |
    pub async fn push(&self, name: impl Into<String>, opts: &ImagePushOpts) -> Result<()> {
        let image = Image::new(self.docker, name);
        image.push(opts).await
    }}

    // api_doc! { Build => Prune
    // /// Clear image build cache.
    // |
    pub async fn clear_cache(&self, opts: &ClearCacheOpts) -> Result<ClearCacheInfo> {
        self.docker
            .post_json(
                construct_ep("/build/prune", opts.serialize()),
                Payload::empty(),
            )
            .await
    }
    // }
}
