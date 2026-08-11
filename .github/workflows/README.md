# GitHub Actions Docker Build Workflow

This workflow automatically builds and pushes Docker images to Docker Hub on push to main/master branches or on manual trigger.

## Required GitHub Secrets

Configure these secrets in your GitHub repository settings (Settings → Secrets and variables → Actions):

### Docker Hub Secrets

- **`DOCKER_USERNAME`**: Your Docker Hub username
- **`DOCKER_TOKEN`**: Docker Hub Access Token (not password)
  - Create at: Docker Hub → Account Settings → Security → New Access Token
  - Required scopes: Read, Write, Delete
- **`DOCKER_IMAGE_PATH`**: (Optional) Your Docker Hub image path (e.g., `your-username/your-project`)
  - Defaults to `github.repository` if not set

## Workflow Triggers

The workflow runs on:
- Push to `main` or `master` branches
- Pull requests to `main` or `master` branches
- Manual trigger via GitHub Actions UI

## Built Images

The workflow builds and pushes two images:

1. **Big Paragon**: Main application image
   - Registry path: `docker.io/{DOCKER_IMAGE_PATH}/big-paragon`
   - Tags: branch name, SHA, semver, latest

2. **Hami**: Hami service image
   - Registry path: `docker.io/{DOCKER_IMAGE_PATH}/hami`
   - Tags: branch name, SHA, semver, latest

## Using the Images

After the workflow completes, you can pull the images from Docker Hub:

```bash
# Pull Big Paragon image
docker pull data-drive-paragon/bp-suite/big-paragon:latest

# Pull Hami image
docker pull data-drive-paragon/bp-suite/hami:latest
```

## Local Testing

To test the workflow locally before pushing:

```bash
# Install act (GitHub Actions runner)
brew install act  # macOS
# or
curl https://raw.githubusercontent.com/nektos/act/master/install.sh | sudo bash

# Run the workflow
act push
```

## Manual Trigger

To manually trigger the workflow:
1. Go to Actions tab in GitHub
2. Select "Build and Push Docker Images to Docker Hub"
3. Click "Run workflow"
4. Select branch and click "Run workflow"

## GitHub Releases

When you create a GitHub Release, the workflow automatically:
1. Builds and pushes Docker images to Docker Hub
2. Exports the images as tarball files
3. Uploads the tarballs as release assets

### Creating a Release

To create a release with Docker image artifacts:
1. Go to Releases page in GitHub
2. Click "Create a new release"
3. Choose a tag version (e.g., v1.0.0)
4. Add release notes
5. Click "Publish release"

The workflow will automatically attach:
- `big-paragon-image.tar.gz` - Compressed Big Paragon Docker image
- `hami-image.tar.gz` - Compressed Hami Docker image

### Loading Images from Release

To load Docker images from a release:

```bash
# Download the image from release assets
wget https://github.com/Data-Drive-Paragon/bp-suite/releases/download/v1.0.0/big-paragon-image.tar.gz

# Load into Docker
docker load -i big-paragon-image.tar.gz

# Tag and use
docker tag <image-id> big-paragon:latest
docker run big-paragon:latest
```
