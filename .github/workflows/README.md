# GitHub Actions Docker Build Workflow

This workflow automatically builds and pushes Docker images to GitLab Container Registry on push to main/master branches or on manual trigger.

## Required GitHub Secrets

Configure these secrets in your GitHub repository settings (Settings → Secrets and variables → Actions):

### GitLab Registry Secrets

- **`GITLAB_USERNAME`**: Your GitLab username
- **`GITLAB_TOKEN`**: GitLab Personal Access Token with `read_registry` and `write_registry` scopes
  - Create at: GitLab → User Settings → Access Tokens
- **`GITLAB_IMAGE_PATH`**: Your GitLab project path (e.g., `your-username/your-project`)

## Workflow Triggers

The workflow runs on:
- Push to `main` or `master` branches
- Pull requests to `main` or `master` branches
- Manual trigger via GitHub Actions UI

## Built Images

The workflow builds and pushes two images:

1. **Big Paragon**: Main application image
   - Registry path: `registry.gitlab.com/{GITLAB_IMAGE_PATH}/big-paragon`
   - Tags: branch name, SHA, semver, latest

2. **Hami**: Hami service image
   - Registry path: `registry.gitlab.com/{GITLAB_IMAGE_PATH}/hami`
   - Tags: branch name, SHA, semver, latest

## Using the Images

After the workflow completes, you can pull the images from GitLab:

```bash
# Pull Big Paragon image
docker pull registry.gitlab.com/{GITLAB_IMAGE_PATH}/big-paragon:latest

# Pull Hami image
docker pull registry.gitlab.com/{GITLAB_IMAGE_PATH}/hami:latest
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
