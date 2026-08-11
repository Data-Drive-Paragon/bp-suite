# GitHub Actions Docker Build Workflow

This workflow automatically builds and pushes Docker images to GitHub Container Registry (GHCR) on push to main/master branches or on manual trigger.

## Required GitHub Secrets

No additional secrets are required! The workflow uses the built-in `GITHUB_TOKEN` which is automatically provided by GitHub Actions with the necessary permissions for pushing to the repository's container registry.

## Workflow Triggers

The workflow runs on:
- Push to `main` or `master` branches
- Pull requests to `main` or `master` branches
- Manual trigger via GitHub Actions UI

## Built Images

The workflow builds and pushes two images:

1. **Big Paragon**: Main application image
   - Registry path: `ghcr.io/{REPOSITORY_OWNER}/{REPOSITORY_NAME}/big-paragon`
   - Tags: branch name, SHA, semver, latest

2. **Hami**: Hami service image
   - Registry path: `ghcr.io/{REPOSITORY_OWNER}/{REPOSITORY_NAME}/hami`
   - Tags: branch name, SHA, semver, latest

## Using the Images

After the workflow completes, you can pull the images from GitHub Container Registry:

```bash
# Pull Big Paragon image
docker pull ghcr.io/Data-Drive-Paragon/bp-suite/big-paragon:latest

# Pull Hami image
docker pull ghcr.io/Data-Drive-Paragon/bp-suite/hami:latest
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
2. Select "Build and Push Docker Images to GitLab Registry"
3. Click "Run workflow"
4. Select branch and click "Run workflow"
